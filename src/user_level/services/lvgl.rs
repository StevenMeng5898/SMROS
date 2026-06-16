//! SMROS-native LVGL-style UI porting layer.
//!
//! The real upstream LVGL runtime is a C library that needs a tick source,
//! display flush hook, and input callbacks. SMROS does not yet expose a
//! framebuffer device, so this module provides the same practical seams for the
//! current kernel: CPU raster widgets, PPM display flushes into FxFS, and ANSI
//! terminal widgets for serial input surfaces.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::user_level::fxfs;

pub const LVGL_PORT_NAME: &str = "smros-lvgl-native";
pub const LVGL_COMPAT_VERSION: &str = "lvgl-9-style";
pub const LVGL_ROOT: &str = "/data/lvgl";
pub const LVGL_DEMO_PPM_PATH: &str = "/data/lvgl/workbench.ppm";
pub const LVGL_DEMO_WIDTH: usize = 480;
pub const LVGL_DEMO_HEIGHT: usize = 288;

const ARC_POINTS: &[(isize, isize)] = &[
    (-707, 707),
    (-866, 500),
    (-966, 259),
    (-1000, 0),
    (-966, -259),
    (-866, -500),
    (-707, -707),
    (-500, -866),
    (0, -1000),
    (500, -866),
    (707, -707),
    (866, -500),
    (966, -259),
    (1000, 0),
    (966, 259),
    (866, 500),
    (707, 707),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LvglError {
    FxfsInit,
    FxfsPrepare,
    Render,
}

impl LvglError {
    pub fn as_str(self) -> &'static str {
        match self {
            LvglError::FxfsInit => "fxfs init",
            LvglError::FxfsPrepare => "fxfs prepare",
            LvglError::Render => "render",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn mix(self, other: Color, other_percent: usize) -> Color {
        let other_percent = other_percent.min(100);
        let self_percent = 100usize.saturating_sub(other_percent);
        Color {
            r: mix_channel(self.r, other.r, self_percent, other_percent),
            g: mix_channel(self.g, other.g, self_percent, other_percent),
            b: mix_channel(self.b, other.b, self_percent, other_percent),
        }
    }
}

fn mix_channel(a: u8, b: u8, a_percent: usize, b_percent: usize) -> u8 {
    let value = (a as usize)
        .saturating_mul(a_percent)
        .saturating_add((b as usize).saturating_mul(b_percent))
        / 100;
    value.min(255) as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub surface: Color,
    pub surface_alt: Color,
    pub line: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_2: Color,
    pub ok: Color,
    pub warn: Color,
    pub danger: Color,
    pub shadow: Color,
}

pub const COLOR_BLACK: Color = Color::new(3, 8, 10);
pub const COLOR_WHITE: Color = Color::new(244, 251, 248);
pub const COLOR_BG: Color = Color::new(7, 16, 19);
pub const COLOR_SURFACE: Color = Color::new(18, 29, 32);
pub const COLOR_SURFACE_ALT: Color = Color::new(24, 39, 42);
pub const COLOR_LINE: Color = Color::new(54, 77, 82);
pub const COLOR_MUTED: Color = Color::new(137, 158, 163);
pub const COLOR_TEAL: Color = Color::new(0, 166, 166);
pub const COLOR_AMBER: Color = Color::new(255, 176, 0);
pub const COLOR_GREEN: Color = Color::new(72, 190, 123);
pub const COLOR_RED: Color = Color::new(226, 76, 76);
pub const COLOR_BLUE: Color = Color::new(95, 166, 230);
pub const COLOR_WORK_BG: Color = Color::new(20, 24, 28);
pub const COLOR_WORK_SURFACE: Color = Color::new(34, 40, 46);
pub const COLOR_WORK_ALT: Color = Color::new(45, 52, 60);
pub const COLOR_WORK_LINE: Color = Color::new(76, 88, 101);
pub const COLOR_WORK_TEXT: Color = Color::new(238, 244, 248);
pub const COLOR_WORK_MUTED: Color = Color::new(154, 166, 176);
pub const COLOR_WORK_ACCENT: Color = Color::new(37, 138, 255);
pub const COLOR_WORK_ACCENT_2: Color = Color::new(0, 177, 150);

pub const AUTOMOTIVE_THEME: Theme = Theme {
    bg: COLOR_BG,
    surface: COLOR_SURFACE,
    surface_alt: COLOR_SURFACE_ALT,
    line: COLOR_LINE,
    text: COLOR_WHITE,
    muted: COLOR_MUTED,
    accent: COLOR_TEAL,
    accent_2: COLOR_BLUE,
    ok: COLOR_GREEN,
    warn: COLOR_AMBER,
    danger: COLOR_RED,
    shadow: COLOR_BLACK,
};

pub const WORKBENCH_THEME: Theme = Theme {
    bg: COLOR_WORK_BG,
    surface: COLOR_WORK_SURFACE,
    surface_alt: COLOR_WORK_ALT,
    line: COLOR_WORK_LINE,
    text: COLOR_WORK_TEXT,
    muted: COLOR_WORK_MUTED,
    accent: COLOR_WORK_ACCENT,
    accent_2: COLOR_WORK_ACCENT_2,
    ok: COLOR_GREEN,
    warn: COLOR_AMBER,
    danger: COLOR_RED,
    shadow: Color::new(10, 13, 16),
};

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, background: Color) -> Self {
        Self::try_new(width, height, background).unwrap_or_else(|_| Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        })
    }

    pub fn try_new(width: usize, height: usize, background: Color) -> Result<Self, ()> {
        let bytes = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(())?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(bytes).map_err(|_| ())?;
        pixels.resize(bytes, 0);
        let mut canvas = Self {
            width,
            height,
            pixels,
        };
        canvas.fill_rect(
            Rect {
                x: 0,
                y: 0,
                w: width,
                h: height,
            },
            background,
        );
        Ok(canvas)
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = (y * self.width + x) * 3;
        self.pixels[offset] = color.r;
        self.pixels[offset + 1] = color.g;
        self.pixels[offset + 2] = color.b;
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x_end = rect.x.saturating_add(rect.w).min(self.width);
        let y_end = rect.y.saturating_add(rect.h).min(self.height);
        let mut y = rect.y.min(self.height);
        while y < y_end {
            let mut x = rect.x.min(self.width);
            while x < x_end {
                self.set_pixel(x, y, color);
                x += 1;
            }
            y += 1;
        }
    }

    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        self.fill_rect(
            Rect {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: 1,
            },
            color,
        );
        self.fill_rect(
            Rect {
                x: rect.x,
                y: rect.y.saturating_add(rect.h.saturating_sub(1)),
                w: rect.w,
                h: 1,
            },
            color,
        );
        self.fill_rect(
            Rect {
                x: rect.x,
                y: rect.y,
                w: 1,
                h: rect.h,
            },
            color,
        );
        self.fill_rect(
            Rect {
                x: rect.x.saturating_add(rect.w.saturating_sub(1)),
                y: rect.y,
                w: 1,
                h: rect.h,
            },
            color,
        );
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: usize, color: Color) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        let radius = radius.min(rect.w / 2).min(rect.h / 2);
        if radius == 0 {
            self.fill_rect(rect, color);
            return;
        }

        let x_end = rect.x.saturating_add(rect.w).min(self.width);
        let y_end = rect.y.saturating_add(rect.h).min(self.height);
        let mut y = rect.y.min(self.height);
        while y < y_end {
            let mut x = rect.x.min(self.width);
            while x < x_end {
                let lx = x.saturating_sub(rect.x);
                let ly = y.saturating_sub(rect.y);
                if rounded_rect_contains(lx, ly, rect.w, rect.h, radius) {
                    self.set_pixel(x, y, color);
                }
                x += 1;
            }
            y += 1;
        }
    }

    pub fn draw_line(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, color: Color) {
        self.draw_line_signed(x0 as isize, y0 as isize, x1 as isize, y1 as isize, color);
    }

    pub fn draw_thick_line(
        &mut self,
        x0: isize,
        y0: isize,
        x1: isize,
        y1: isize,
        color: Color,
        radius: isize,
    ) {
        let radius = radius.max(0);
        let mut oy = -radius;
        while oy <= radius {
            let mut ox = -radius;
            while ox <= radius {
                if ox * ox + oy * oy <= radius * radius {
                    self.draw_line_signed(x0 + ox, y0 + oy, x1 + ox, y1 + oy, color);
                }
                ox += 1;
            }
            oy += 1;
        }
    }

    pub fn draw_line_signed(
        &mut self,
        mut x0: isize,
        mut y0: isize,
        x1: isize,
        y1: isize,
        color: Color,
    ) {
        let dx = abs_isize(x1 - x0);
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -abs_isize(y1 - y0);
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            if x0 >= 0 && y0 >= 0 {
                self.set_pixel(x0 as usize, y0 as usize, color);
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = err * 2;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn fill_circle(&mut self, cx: isize, cy: isize, radius: usize, color: Color) {
        let r = radius as isize;
        let r_sq = r * r;
        let mut y = cy - r;
        while y <= cy + r {
            let mut x = cx - r;
            while x <= cx + r {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= r_sq && x >= 0 && y >= 0 {
                    self.set_pixel(x as usize, y as usize, color);
                }
                x += 1;
            }
            y += 1;
        }
    }

    pub fn draw_ring(&mut self, cx: usize, cy: usize, inner: usize, outer: usize, color: Color) {
        let outer_sq = outer.saturating_mul(outer) as isize;
        let inner_sq = inner.saturating_mul(inner) as isize;
        let x0 = cx.saturating_sub(outer).min(self.width);
        let x1 = cx.saturating_add(outer).min(self.width.saturating_sub(1));
        let y0 = cy.saturating_sub(outer).min(self.height);
        let y1 = cy.saturating_add(outer).min(self.height.saturating_sub(1));
        let mut y = y0;
        while y <= y1 {
            let mut x = x0;
            while x <= x1 {
                let dx = x as isize - cx as isize;
                let dy = y as isize - cy as isize;
                let dist = dx * dx + dy * dy;
                if dist >= inner_sq && dist <= outer_sq {
                    self.set_pixel(x, y, color);
                }
                x += 1;
            }
            y += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LvglPortInfo {
    pub name: &'static str,
    pub compat_version: &'static str,
    pub display_backend: &'static str,
    pub input_backend: &'static str,
    pub tick_backend: &'static str,
    pub draw_buffer_bytes: usize,
    pub widgets: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LvglDemoRender {
    pub image_path: &'static str,
    pub preview: String,
    pub width: usize,
    pub height: usize,
    pub image_bytes: usize,
    pub pixel_bytes: usize,
    pub widgets: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LvglTestReport {
    pub port_ok: bool,
    pub display_flush_ok: bool,
    pub input_ok: bool,
    pub widgets_ok: bool,
    pub fxfs_ok: bool,
    pub render: LvglDemoRender,
}

impl LvglTestReport {
    pub fn passed(&self) -> bool {
        self.port_ok && self.display_flush_ok && self.input_ok && self.widgets_ok && self.fxfs_ok
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterSpec<'a> {
    pub cx: usize,
    pub cy: usize,
    pub radius: usize,
    pub value: usize,
    pub max: usize,
    pub label: &'a str,
    pub unit: &'a str,
    pub accent: Color,
}

pub fn init() -> bool {
    prepare_storage().is_ok()
}

pub fn info() -> LvglPortInfo {
    LvglPortInfo {
        name: LVGL_PORT_NAME,
        compat_version: LVGL_COMPAT_VERSION,
        display_backend: "fxfs-ppm-flush",
        input_backend: "serial-keypad-pointer",
        tick_backend: "scheduler-ticks",
        draw_buffer_bytes: LVGL_DEMO_WIDTH
            .saturating_mul(LVGL_DEMO_HEIGHT)
            .saturating_mul(3),
        widgets: 9,
    }
}

pub fn render_demo() -> Result<LvglDemoRender, LvglError> {
    prepare_storage()?;
    let mut canvas = Canvas::try_new(LVGL_DEMO_WIDTH, LVGL_DEMO_HEIGHT, WORKBENCH_THEME.bg)
        .map_err(|_| LvglError::Render)?;
    draw_workbench_demo(&mut canvas);
    let preview = render_demo_preview();
    let ppm = try_encode_ppm(canvas.width, canvas.height, canvas.pixels.as_slice())
        .map_err(|_| LvglError::Render)?;
    fxfs::write_file(LVGL_DEMO_PPM_PATH, ppm.as_slice()).map_err(|_| LvglError::FxfsPrepare)?;

    Ok(LvglDemoRender {
        image_path: LVGL_DEMO_PPM_PATH,
        preview,
        width: canvas.width,
        height: canvas.height,
        image_bytes: ppm.len(),
        pixel_bytes: canvas.pixels.len(),
        widgets: 9,
    })
}

pub fn run_full_test() -> Result<LvglTestReport, LvglError> {
    let port = info();
    let render = render_demo()?;
    let port_ok = port.name == LVGL_PORT_NAME
        && port.display_backend == "fxfs-ppm-flush"
        && port.input_backend == "serial-keypad-pointer";
    let display_flush_ok = render.width == LVGL_DEMO_WIDTH
        && render.height == LVGL_DEMO_HEIGHT
        && render.image_bytes > render.pixel_bytes
        && render.preview.contains("SMROS LVGL Workbench");
    let input_ok = serial_input_kind("pointer") == "pointer"
        && serial_input_kind("keypad") == "keypad"
        && serial_input_kind("wheel") == "encoder";
    let widgets_ok = render.widgets >= 9 && render.preview.contains("Arc Meter");
    let fxfs_ok = fxfs::attrs(LVGL_DEMO_PPM_PATH)
        .map(|attrs| attrs.size == render.image_bytes)
        .unwrap_or(false);

    if !(port_ok && display_flush_ok && input_ok && widgets_ok && fxfs_ok) {
        return Err(LvglError::Render);
    }

    Ok(LvglTestReport {
        port_ok,
        display_flush_ok,
        input_ok,
        widgets_ok,
        fxfs_ok,
        render,
    })
}

pub fn smoke_test() -> bool {
    run_full_test()
        .map(|report| report.passed())
        .unwrap_or(false)
}

pub fn serial_input_kind(event: &str) -> &'static str {
    match event {
        "mouse" | "pointer" | "touch" => "pointer",
        "wheel" | "scroll" | "encoder" => "encoder",
        _ => "keypad",
    }
}

pub fn draw_background_grid(canvas: &mut Canvas, theme: Theme, x_step: usize, y_step: usize) {
    canvas.fill_rect(
        Rect {
            x: 0,
            y: 0,
            w: canvas.width,
            h: canvas.height,
        },
        theme.bg,
    );
    let line = theme.bg.mix(theme.line, 28);
    let mut x = 0usize;
    while x < canvas.width {
        canvas.fill_rect(
            Rect {
                x,
                y: 0,
                w: 1,
                h: canvas.height,
            },
            line,
        );
        x = x.saturating_add(x_step.max(1));
    }
    let mut y = 0usize;
    while y < canvas.height {
        canvas.fill_rect(
            Rect {
                x: 0,
                y,
                w: canvas.width,
                h: 1,
            },
            line,
        );
        y = y.saturating_add(y_step.max(1));
    }
}

pub fn draw_panel(canvas: &mut Canvas, rect: Rect, radius: usize, theme: Theme) {
    canvas.fill_rounded_rect(rect, radius, theme.shadow);
    let shifted = Rect {
        x: rect.x,
        y: rect.y.saturating_sub(2),
        w: rect.w,
        h: rect.h,
    };
    canvas.fill_rounded_rect(shifted, radius, theme.surface);
    canvas.stroke_rect(shifted, theme.line);
}

pub fn draw_header(canvas: &mut Canvas, rect: Rect, title: &str, subtitle: &str, theme: Theme) {
    canvas.fill_rounded_rect(rect, 10, theme.surface_alt);
    canvas.stroke_rect(rect, theme.line);
    canvas.fill_rounded_rect(
        Rect {
            x: rect.x + 12,
            y: rect.y + 14,
            w: 8,
            h: rect.h.saturating_sub(28),
        },
        4,
        theme.accent,
    );
    let title_scale = if rect.h < 70 || rect.w < 600 { 2 } else { 3 };
    let subtitle_y = if rect.h < 70 {
        rect.y + rect.h.saturating_sub(22)
    } else {
        rect.y + 52
    };
    draw_label(
        canvas,
        rect.x + 32,
        rect.y + 18,
        title,
        theme.text,
        title_scale,
    );
    draw_label(canvas, rect.x + 34, subtitle_y, subtitle, theme.muted, 1);
}

pub fn draw_label(canvas: &mut Canvas, x: usize, y: usize, text: &str, color: Color, scale: usize) {
    let scale = scale.max(1);
    let mut cursor = x;
    for byte in sanitize_ascii(text).bytes() {
        if byte == b' ' {
            cursor += 4 * scale;
        } else {
            draw_glyph(canvas, cursor, y, byte, color, scale);
            cursor += 6 * scale;
        }
        if cursor >= canvas.width.saturating_sub(6 * scale) {
            break;
        }
    }
}

pub fn draw_label_centered(
    canvas: &mut Canvas,
    cx: usize,
    y: usize,
    text: &str,
    color: Color,
    scale: usize,
) {
    let width = text_width(text, scale);
    let x = cx.saturating_sub(width / 2);
    draw_label(canvas, x, y, text, color, scale);
}

pub fn draw_button(canvas: &mut Canvas, rect: Rect, label: &str, active: bool, theme: Theme) {
    let fill = if active {
        theme.accent
    } else {
        theme.surface_alt
    };
    let text = if active { COLOR_WHITE } else { theme.text };
    canvas.fill_rounded_rect(rect, 8, fill);
    canvas.stroke_rect(rect, if active { theme.accent_2 } else { theme.line });
    draw_label_centered(canvas, rect.x + rect.w / 2, rect.y + 13, label, text, 1);
}

pub fn draw_text_area(
    canvas: &mut Canvas,
    rect: Rect,
    title: &str,
    body: &str,
    focused: bool,
    theme: Theme,
) {
    canvas.fill_rounded_rect(rect, 8, theme.surface_alt);
    canvas.stroke_rect(rect, if focused { theme.accent } else { theme.line });
    draw_label(canvas, rect.x + 12, rect.y + 10, title, theme.muted, 1);
    draw_wrapped_labels(
        canvas,
        rect.x + 12,
        rect.y + 32,
        rect.w.saturating_sub(24),
        rect.h.saturating_sub(38),
        body,
        theme.text,
        1,
    );
}

pub fn draw_progress_bar(
    canvas: &mut Canvas,
    rect: Rect,
    value: usize,
    max: usize,
    color: Color,
    theme: Theme,
) {
    canvas.fill_rounded_rect(rect, 5, theme.shadow);
    canvas.stroke_rect(rect, theme.line);
    if max == 0 || rect.w <= 6 || rect.h <= 6 {
        return;
    }
    let fill = value.min(max).saturating_mul(rect.w.saturating_sub(6)) / max;
    canvas.fill_rounded_rect(
        Rect {
            x: rect.x + 3,
            y: rect.y + 3,
            w: fill,
            h: rect.h - 6,
        },
        3,
        color,
    );
}

pub fn draw_meter_card(canvas: &mut Canvas, spec: MeterSpec<'_>, theme: Theme) {
    let pad_x = if spec.radius < 96 { 24 } else { 44 };
    let pad_top = if spec.radius < 96 { 30 } else { 48 };
    let pad_bottom = if spec.radius < 96 { 38 } else { 76 };
    draw_panel(
        canvas,
        Rect {
            x: spec.cx.saturating_sub(spec.radius + pad_x),
            y: spec.cy.saturating_sub(spec.radius + pad_top),
            w: spec.radius.saturating_mul(2).saturating_add(pad_x * 2),
            h: spec
                .radius
                .saturating_mul(2)
                .saturating_add(pad_top)
                .saturating_add(pad_bottom),
        },
        14,
        theme,
    );
    canvas.draw_ring(
        spec.cx,
        spec.cy,
        spec.radius.saturating_sub(8),
        spec.radius + 8,
        theme.line,
    );
    draw_arc_progress(
        canvas,
        spec.cx,
        spec.cy,
        spec.radius,
        spec.value.min(spec.max),
        spec.max,
        spec.accent,
        5,
    );
    draw_meter_ticks(canvas, spec.cx, spec.cy, spec.radius, theme.line);

    let needle = arc_point(
        spec.cx,
        spec.cy,
        spec.radius.saturating_sub(42),
        spec.value.min(spec.max),
        spec.max,
    );
    canvas.draw_thick_line(
        spec.cx as isize,
        spec.cy as isize,
        needle.0,
        needle.1,
        theme.text,
        2,
    );
    canvas.fill_circle(spec.cx as isize, spec.cy as isize, 10, spec.accent);
    canvas.fill_circle(spec.cx as isize, spec.cy as isize, 4, theme.shadow);

    let mut value_text = String::new();
    append_usize(&mut value_text, spec.value, 0);
    let value_scale = if spec.radius < 72 {
        2
    } else if spec.radius < 112 {
        3
    } else {
        5
    };
    let label_scale = if spec.radius < 112 { 1 } else { 2 };
    draw_label_centered(
        canvas,
        spec.cx,
        spec.cy.saturating_sub(26),
        value_text.as_str(),
        theme.text,
        value_scale,
    );
    draw_label_centered(
        canvas,
        spec.cx,
        spec.cy + 30,
        spec.unit,
        theme.muted,
        label_scale,
    );
    draw_label_centered(
        canvas,
        spec.cx,
        spec.cy + if spec.radius < 112 { 56 } else { 74 },
        spec.label,
        spec.accent,
        label_scale,
    );
}

pub fn draw_arc_progress(
    canvas: &mut Canvas,
    cx: usize,
    cy: usize,
    radius: usize,
    value: usize,
    max: usize,
    color: Color,
    thickness: isize,
) {
    if max == 0 {
        return;
    }
    let steps = 96usize;
    let filled = value.saturating_mul(steps) / max;
    let mut previous = arc_point(cx, cy, radius, 0, steps);
    let mut step = 1usize;
    while step <= filled {
        let point = arc_point(cx, cy, radius, step, steps);
        canvas.draw_thick_line(previous.0, previous.1, point.0, point.1, color, thickness);
        previous = point;
        step += 1;
    }
}

pub fn draw_chart(
    canvas: &mut Canvas,
    rect: Rect,
    values: &[usize],
    max: usize,
    color: Color,
    theme: Theme,
) {
    canvas.fill_rounded_rect(rect, 8, theme.surface_alt);
    canvas.stroke_rect(rect, theme.line);
    if values.len() < 2 || max == 0 || rect.w <= 8 || rect.h <= 8 {
        return;
    }
    let inner_x = rect.x + 6;
    let inner_y = rect.y + 6;
    let inner_w = rect.w - 12;
    let inner_h = rect.h - 12;
    let mut gx = 1usize;
    while gx < 4 {
        let x = inner_x + gx * inner_w / 4;
        canvas.fill_rect(
            Rect {
                x,
                y: inner_y,
                w: 1,
                h: inner_h,
            },
            theme.line.mix(theme.surface, 55),
        );
        gx += 1;
    }
    let mut index = 1usize;
    while index < values.len() {
        let x0 = inner_x + (index - 1) * inner_w / (values.len() - 1);
        let x1 = inner_x + index * inner_w / (values.len() - 1);
        let y0 = inner_y + inner_h.saturating_sub(values[index - 1].min(max) * inner_h / max);
        let y1 = inner_y + inner_h.saturating_sub(values[index].min(max) * inner_h / max);
        canvas.draw_thick_line(x0 as isize, y0 as isize, x1 as isize, y1 as isize, color, 2);
        index += 1;
    }
}

pub fn encode_ppm(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    try_encode_ppm(width, height, pixels).unwrap_or_else(|_| Vec::new())
}

pub fn try_encode_ppm(width: usize, height: usize, pixels: &[u8]) -> Result<Vec<u8>, ()> {
    let capacity = 16usize
        .saturating_add(decimal_len(width))
        .saturating_add(decimal_len(height))
        .saturating_add(pixels.len());
    let mut out = Vec::new();
    out.try_reserve_exact(capacity).map_err(|_| ())?;
    out.extend_from_slice(b"P6\n");
    push_decimal_bytes(&mut out, width);
    out.push(b' ');
    push_decimal_bytes(&mut out, height);
    out.extend_from_slice(b"\n255\n");
    out.extend_from_slice(pixels);
    Ok(out)
}

pub fn push_ansi_style(out: &mut String, fg: Color, bg: Color) {
    out.push_str("\x1b[38;2;");
    append_usize(out, fg.r as usize, 0);
    out.push(';');
    append_usize(out, fg.g as usize, 0);
    out.push(';');
    append_usize(out, fg.b as usize, 0);
    out.push_str("m\x1b[48;2;");
    append_usize(out, bg.r as usize, 0);
    out.push(';');
    append_usize(out, bg.g as usize, 0);
    out.push(';');
    append_usize(out, bg.b as usize, 0);
    out.push('m');
}

pub fn push_ansi_line(out: &mut String, text: &str, fg: Color, bg: Color, width: usize) {
    push_ansi_style(out, fg, bg);
    push_fixed_ascii(out, text, width);
    out.push_str("\x1b[0m\n");
}

pub fn push_meter_text(out: &mut String, value: usize, max: usize, width: usize) {
    out.push('[');
    let filled = if max == 0 {
        0
    } else {
        value.min(max).saturating_mul(width) / max
    };
    let mut index = 0usize;
    while index < width {
        out.push(if index < filled { '#' } else { '-' });
        index += 1;
    }
    out.push(']');
}

pub fn push_fixed_ascii(out: &mut String, text: &str, width: usize) {
    let mut written = 0usize;
    for byte in text.bytes() {
        if written >= width {
            return;
        }
        if byte == b'\n' || byte == b'\r' {
            out.push(' ');
        } else if (0x20..=0x7e).contains(&byte) {
            out.push(byte as char);
        } else {
            out.push('?');
        }
        written += 1;
    }
    while written < width {
        out.push(' ');
        written += 1;
    }
}

pub fn append_usize(out: &mut String, mut value: usize, min_width: usize) {
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    if value == 0 {
        buf[len] = b'0';
        len += 1;
    }
    while value != 0 && len < buf.len() {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len < min_width && len < buf.len() {
        buf[len] = b'0';
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
}

fn prepare_storage() -> Result<(), LvglError> {
    if !fxfs::init() {
        return Err(LvglError::FxfsInit);
    }
    fxfs::create_dir("/data").map_err(|_| LvglError::FxfsPrepare)?;
    fxfs::create_dir(LVGL_ROOT).map_err(|_| LvglError::FxfsPrepare)?;
    Ok(())
}

fn draw_workbench_demo(canvas: &mut Canvas) {
    let theme = WORKBENCH_THEME;
    draw_background_grid(canvas, theme, 48, 36);
    draw_header(
        canvas,
        Rect {
            x: 16,
            y: 12,
            w: 448,
            h: 58,
        },
        "SMROS LVGL Workbench",
        "FxFS PPM flush / serial input / scheduler ticks",
        theme,
    );
    draw_meter_card(
        canvas,
        MeterSpec {
            cx: 110,
            cy: 162,
            radius: 58,
            value: 68,
            max: 100,
            label: "CPU",
            unit: "LOAD",
            accent: theme.accent,
        },
        theme,
    );
    draw_meter_card(
        canvas,
        MeterSpec {
            cx: 260,
            cy: 162,
            radius: 58,
            value: 42,
            max: 100,
            label: "IO",
            unit: "QUEUE",
            accent: theme.accent_2,
        },
        theme,
    );
    draw_text_area(
        canvas,
        Rect {
            x: 344,
            y: 88,
            w: 112,
            h: 102,
        },
        "Textarea",
        "Hermes and QML use SMROS LVGL widgets.",
        true,
        theme,
    );
    draw_label(canvas, 348, 204, "Controls", theme.muted, 1);
    draw_button(
        canvas,
        Rect {
            x: 348,
            y: 224,
            w: 48,
            h: 28,
        },
        "ASK",
        true,
        theme,
    );
    draw_button(
        canvas,
        Rect {
            x: 408,
            y: 224,
            w: 48,
            h: 28,
        },
        "TEST",
        false,
        theme,
    );
    draw_panel(
        canvas,
        Rect {
            x: 26,
            y: 240,
            w: 178,
            h: 32,
        },
        10,
        theme,
    );
    draw_label(canvas, 38, 250, "Buffer", theme.text, 1);
    draw_progress_bar(
        canvas,
        Rect {
            x: 104,
            y: 251,
            w: 82,
            h: 12,
        },
        74,
        100,
        theme.ok,
        theme,
    );
    draw_label(canvas, 224, 236, "Frame time", theme.muted, 1);
    draw_chart(
        canvas,
        Rect {
            x: 224,
            y: 252,
            w: 232,
            h: 26,
        },
        &[12, 16, 14, 22, 18, 19, 15, 17, 13, 16],
        28,
        theme.warn,
        theme,
    );
}

fn render_demo_preview() -> String {
    let mut out = String::new();
    push_ansi_line(
        &mut out,
        " SMROS LVGL Workbench                                                        ",
        WORKBENCH_THEME.text,
        WORKBENCH_THEME.accent,
        78,
    );
    push_ansi_line(
        &mut out,
        " FxFS PPM display flush | serial pointer/keypad input | scheduler ticks       ",
        WORKBENCH_THEME.text,
        WORKBENCH_THEME.surface,
        78,
    );
    push_ansi_line(
        &mut out,
        " +---------------- Arc Meter ----------------+ +--------- Widgets ---------+  ",
        WORKBENCH_THEME.line,
        WORKBENCH_THEME.bg,
        78,
    );
    let mut meter = String::from(" | CPU ");
    push_meter_text(&mut meter, 68, 100, 22);
    meter.push_str("  IO ");
    push_meter_text(&mut meter, 42, 100, 22);
    meter.push_str(" | | Textarea Button Bar |  ");
    push_ansi_line(
        &mut out,
        meter.as_str(),
        WORKBENCH_THEME.text,
        WORKBENCH_THEME.bg,
        78,
    );
    push_ansi_line(
        &mut out,
        " +-------------------------------------------+ +--------------------------+  ",
        WORKBENCH_THEME.line,
        WORKBENCH_THEME.bg,
        78,
    );
    let mut bottom = String::from(" Draw buffer ");
    push_meter_text(&mut bottom, 74, 100, 30);
    bottom.push_str("  chart: 12 16 14 22 18 19 15");
    push_ansi_line(
        &mut out,
        bottom.as_str(),
        WORKBENCH_THEME.ok,
        WORKBENCH_THEME.bg,
        78,
    );
    out.push_str("\x1b[0m");
    out
}

fn draw_meter_ticks(canvas: &mut Canvas, cx: usize, cy: usize, radius: usize, color: Color) {
    let mut index = 0usize;
    while index <= 8 {
        let value = index * 1000;
        let outer = arc_point(cx, cy, radius + 18, value, 8000);
        let inner = arc_point(cx, cy, radius + 4, value, 8000);
        canvas.draw_thick_line(inner.0, inner.1, outer.0, outer.1, color, 1);
        index += 1;
    }
}

fn arc_point(cx: usize, cy: usize, radius: usize, value: usize, max: usize) -> (isize, isize) {
    if max == 0 {
        return (cx as isize, cy as isize);
    }
    let last = ARC_POINTS.len().saturating_sub(1);
    let scaled = value.min(max).saturating_mul(last).saturating_mul(1000) / max;
    let index = (scaled / 1000).min(last);
    let frac = (scaled % 1000) as isize;
    let (x0, y0) = ARC_POINTS[index];
    let (x1, y1) = if index < last {
        ARC_POINTS[index + 1]
    } else {
        ARC_POINTS[index]
    };
    let x = x0 + ((x1 - x0) * frac) / 1000;
    let y = y0 + ((y1 - y0) * frac) / 1000;
    (
        cx as isize + (x * radius as isize) / 1000,
        cy as isize + (y * radius as isize) / 1000,
    )
}

fn draw_wrapped_labels(
    canvas: &mut Canvas,
    x: usize,
    y: usize,
    width_px: usize,
    height_px: usize,
    text: &str,
    color: Color,
    scale: usize,
) {
    let char_width = 6usize.saturating_mul(scale.max(1));
    let row_height = 9usize.saturating_mul(scale.max(1));
    let max_cols = (width_px / char_width).max(1);
    let max_rows = (height_px / row_height).max(1);
    let mut row = 0usize;
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len().saturating_add(1).saturating_add(word.len()) > max_cols {
            if row >= max_rows {
                return;
            }
            draw_label(canvas, x, y + row * row_height, line.as_str(), color, scale);
            row += 1;
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() && row < max_rows {
        draw_label(canvas, x, y + row * row_height, line.as_str(), color, scale);
    }
}

fn rounded_rect_contains(x: usize, y: usize, w: usize, h: usize, radius: usize) -> bool {
    if x >= radius && x < w.saturating_sub(radius) {
        return true;
    }
    if y >= radius && y < h.saturating_sub(radius) {
        return true;
    }

    let cx = if x < radius {
        radius.saturating_sub(1)
    } else {
        w.saturating_sub(radius)
    };
    let cy = if y < radius {
        radius.saturating_sub(1)
    } else {
        h.saturating_sub(radius)
    };
    let dx = x.max(cx) - x.min(cx);
    let dy = y.max(cy) - y.min(cy);
    dx.saturating_mul(dx) + dy.saturating_mul(dy) <= radius.saturating_mul(radius)
}

fn draw_glyph(canvas: &mut Canvas, x: usize, y: usize, byte: u8, color: Color, scale: usize) {
    let glyph = glyph_rows(byte);
    for (row, bits) in glyph.iter().enumerate() {
        let mut col = 0usize;
        while col < 5 {
            if bits & (1 << (4 - col)) != 0 {
                canvas.fill_rect(
                    Rect {
                        x: x + col * scale,
                        y: y + row * scale,
                        w: scale,
                        h: scale,
                    },
                    color,
                );
            }
            col += 1;
        }
    }
}

fn glyph_rows(byte: u8) -> [u8; 7] {
    match byte.to_ascii_uppercase() {
        b'0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        b'1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        b'2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        b'3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        b'4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        b'5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        b'6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        b'7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        b'8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        b'9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b11100,
        ],
        b'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        b'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        b'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        b'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        b'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        b'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        b'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        b'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        b'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        b'J' => [
            0b00001, 0b00001, 0b00001, 0b00001, 0b10001, 0b10001, 0b01110,
        ],
        b'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        b'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        b'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        b'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        b'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        b'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        b'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        b'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        b'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        b'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        b'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        b'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        b'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        b'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        b'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        b'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        b'-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        b'/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        b'.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        b':' => [
            0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000,
        ],
        b'<' => [
            0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010,
        ],
        b'>' => [
            0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000,
        ],
        b'%' => [
            0b11001, 0b11010, 0b00100, 0b01000, 0b10110, 0b00110, 0b00000,
        ],
        b'+' => [
            0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
        ],
        b'|' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        _ => [
            0b11111, 0b10001, 0b00010, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
    }
}

fn text_width(text: &str, scale: usize) -> usize {
    sanitize_ascii(text).len().saturating_mul(6 * scale.max(1))
}

fn sanitize_ascii(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte == b'\n' || byte == b'\r' {
            out.push(' ');
        } else if (0x20..=0x7e).contains(&byte) {
            out.push(byte as char);
        } else {
            out.push('?');
        }
    }
    out
}

fn push_decimal_bytes(out: &mut Vec<u8>, mut value: usize) {
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    if value == 0 {
        out.push(b'0');
        return;
    }
    while value > 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len]);
    }
}

fn decimal_len(mut value: usize) -> usize {
    let mut len = 1usize;
    while value >= 10 {
        value /= 10;
        len += 1;
    }
    len
}

fn abs_isize(value: isize) -> isize {
    if value < 0 {
        -value
    } else {
        value
    }
}

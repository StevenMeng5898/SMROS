//! Small native HTML UI renderer for SMROS shell surfaces.
//!
//! It accepts the limited HTML that SMROS services emit, extracts semantic
//! widgets, and renders them into a native serial UI.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlUiError {
    Empty,
    Parse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeHtmlView {
    pub title: String,
    pub rendered: String,
    pub widgets: usize,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuHtmlView {
    pub title: String,
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub ppm: Vec<u8>,
    pub preview: String,
    pub widgets: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MetricWidget {
    label: String,
    value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SkillWidget {
    name: String,
    description: String,
    path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HtmlUiModel {
    title: String,
    metrics: Vec<MetricWidget>,
    pills: Vec<String>,
    skills: Vec<SkillWidget>,
    buttons: Vec<String>,
    prompt: String,
    answer: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

const CPU_UI_WIDTH: usize = 720;
const CPU_UI_HEIGHT: usize = 420;
const ANSI_PREVIEW_COLS: usize = 78;
const COLOR_BG: Rgb = Rgb::new(242, 245, 248);
const COLOR_SURFACE: Rgb = Rgb::new(255, 255, 255);
const COLOR_MUTED_SURFACE: Rgb = Rgb::new(248, 250, 252);
const COLOR_INK: Rgb = Rgb::new(23, 32, 38);
const COLOR_MUTED: Rgb = Rgb::new(88, 99, 108);
const COLOR_LINE: Rgb = Rgb::new(208, 216, 224);
const COLOR_ACCENT: Rgb = Rgb::new(11, 107, 203);
const COLOR_ACCENT_DARK: Rgb = Rgb::new(7, 72, 137);
const COLOR_OK: Rgb = Rgb::new(20, 125, 74);
const COLOR_DARK: Rgb = Rgb::new(16, 24, 32);
const COLOR_LIGHT_TEXT: Rgb = Rgb::new(234, 242, 248);
const COLOR_SHADOW: Rgb = Rgb::new(224, 230, 237);

impl Rgb {
    const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl Canvas {
    fn new(width: usize, height: usize, background: Rgb) -> Self {
        let mut pixels = Vec::new();
        pixels.resize(width.saturating_mul(height).saturating_mul(3), 0);
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
        canvas
    }

    fn set_pixel(&mut self, x: usize, y: usize, color: Rgb) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = (y * self.width + x) * 3;
        self.pixels[offset] = color.r;
        self.pixels[offset + 1] = color.g;
        self.pixels[offset + 2] = color.b;
    }

    fn fill_rect(&mut self, rect: Rect, color: Rgb) {
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

    fn stroke_rect(&mut self, rect: Rect, color: Rgb) {
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

    fn fill_rounded_rect(&mut self, rect: Rect, radius: usize, color: Rgb) {
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
}

pub fn render_native_view(html: &str, width: usize) -> Result<NativeHtmlView, HtmlUiError> {
    let model = parse_ui_model(html)?;
    let width = clamp_width(width);
    let mut rendered = String::new();
    push_top(&mut rendered, width, model.title.as_str());
    push_row(&mut rendered, width, "Native HTML UI");
    push_rule(&mut rendered, width);

    if !model.metrics.is_empty() {
        push_row(&mut rendered, width, "Status");
        let mut line = String::new();
        for metric in &model.metrics {
            if !line.is_empty() {
                line.push_str("  ");
            }
            line.push('[');
            line.push_str(metric.label.as_str());
            line.push_str(": ");
            line.push_str(metric.value.as_str());
            line.push(']');
        }
        push_wrapped(&mut rendered, width, line.as_str(), 2);
        push_rule(&mut rendered, width);
    }

    if !model.pills.is_empty() {
        push_row(&mut rendered, width, "Runtime");
        for pill in &model.pills {
            push_wrapped(&mut rendered, width, pill.as_str(), 2);
        }
        push_rule(&mut rendered, width);
    }

    push_row(&mut rendered, width, "Prompt Composer");
    push_wrapped(&mut rendered, width, model.prompt.as_str(), 2);
    if !model.buttons.is_empty() {
        let mut actions = String::from("Actions:");
        for button in &model.buttons {
            actions.push_str(" [");
            actions.push_str(button.as_str());
            actions.push(']');
        }
        push_wrapped(&mut rendered, width, actions.as_str(), 2);
    }
    push_rule(&mut rendered, width);

    push_row(&mut rendered, width, "Response");
    push_wrapped(&mut rendered, width, model.answer.as_str(), 2);
    push_rule(&mut rendered, width);

    if !model.skills.is_empty() {
        push_row(&mut rendered, width, "Skills");
        for skill in &model.skills {
            let mut summary = String::from("* ");
            summary.push_str(skill.name.as_str());
            summary.push_str(": ");
            summary.push_str(skill.description.as_str());
            push_wrapped(&mut rendered, width, summary.as_str(), 2);
            push_wrapped(&mut rendered, width, skill.path.as_str(), 4);
        }
    }

    push_bottom(&mut rendered, width);
    Ok(NativeHtmlView {
        title: model.title,
        rendered,
        widgets: model.metrics.len()
            + model.pills.len()
            + model.skills.len()
            + model.buttons.len()
            + 2,
        width,
    })
}

pub fn render_cpu_view(html: &str) -> Result<CpuHtmlView, HtmlUiError> {
    let model = parse_ui_model(html)?;
    let mut canvas = Canvas::new(CPU_UI_WIDTH, CPU_UI_HEIGHT, COLOR_BG);
    draw_cpu_dashboard(&mut canvas, &model);
    let preview = render_ansi_preview(&model);
    let ppm = encode_ppm(canvas.width, canvas.height, canvas.pixels.as_slice());
    let widgets =
        model.metrics.len() + model.pills.len() + model.skills.len() + model.buttons.len() + 2;
    Ok(CpuHtmlView {
        title: model.title,
        width: canvas.width,
        height: canvas.height,
        pixels: canvas.pixels,
        ppm,
        preview,
        widgets,
    })
}

fn parse_ui_model(html: &str) -> Result<HtmlUiModel, HtmlUiError> {
    if html.trim().is_empty() {
        return Err(HtmlUiError::Empty);
    }

    Ok(HtmlUiModel {
        title: extract_tag(html, "title")
            .or_else(|| extract_tag(html, "h1"))
            .unwrap_or_else(|| String::from("HTML UI")),
        metrics: extract_metrics(html),
        pills: extract_class_texts(html, "pill"),
        skills: extract_skills(html),
        buttons: extract_all_tags(html, "button"),
        prompt: extract_tag(html, "textarea").unwrap_or_else(|| String::from("hermes ask ...")),
        answer: extract_class_texts(html, "answer")
            .into_iter()
            .next()
            .unwrap_or_else(|| String::from("No response content")),
    })
}

fn clamp_width(width: usize) -> usize {
    if width < 56 {
        56
    } else if width > 96 {
        96
    } else {
        width
    }
}

fn draw_cpu_dashboard(canvas: &mut Canvas, model: &HtmlUiModel) {
    draw_panel(
        canvas,
        Rect {
            x: 22,
            y: 18,
            w: 676,
            h: 68,
        },
        COLOR_SURFACE,
    );
    draw_text(canvas, 42, 36, "Hermes Agent", COLOR_INK, 3);
    draw_text(
        canvas,
        42,
        64,
        "CPU-rendered native SMROS UI",
        COLOR_MUTED,
        1,
    );
    draw_pill(canvas, 512, 34, "Gemma", COLOR_ACCENT);
    draw_pill(canvas, 590, 34, "FxFS", COLOR_OK);

    draw_metrics(canvas, model);
    draw_prompt_panel(canvas, model);
    draw_runtime_panel(canvas, model);
    draw_skills_panel(canvas, model);
}

fn draw_metrics(canvas: &mut Canvas, model: &HtmlUiModel) {
    let mut x = 22usize;
    for (index, metric) in model.metrics.iter().take(6).enumerate() {
        let accent = match index {
            0 => COLOR_ACCENT,
            1 => COLOR_OK,
            2 => Rgb::new(145, 80, 16),
            3 => Rgb::new(102, 86, 180),
            4 => Rgb::new(42, 130, 160),
            _ => COLOR_MUTED,
        };
        draw_panel(
            canvas,
            Rect {
                x,
                y: 102,
                w: 106,
                h: 70,
            },
            COLOR_SURFACE,
        );
        canvas.fill_rect(
            Rect {
                x,
                y: 102,
                w: 106,
                h: 4,
            },
            accent,
        );
        draw_text(canvas, x + 12, 122, metric.value.as_str(), COLOR_INK, 2);
        draw_text(canvas, x + 12, 150, metric.label.as_str(), COLOR_MUTED, 1);
        x += 114;
    }
}

fn draw_prompt_panel(canvas: &mut Canvas, model: &HtmlUiModel) {
    draw_panel(
        canvas,
        Rect {
            x: 22,
            y: 192,
            w: 420,
            h: 206,
        },
        COLOR_SURFACE,
    );
    draw_text(canvas, 42, 216, "Prompt Composer", COLOR_INK, 2);
    draw_input(
        canvas,
        Rect {
            x: 42,
            y: 246,
            w: 380,
            h: 58,
        },
        model.prompt.as_str(),
    );
    let mut button_x = 42usize;
    for button in model.buttons.iter().take(2) {
        draw_button(canvas, button_x, 318, button.as_str(), button_x == 42);
        button_x += 110;
    }
    draw_text(canvas, 42, 360, "Response", COLOR_INK, 1);
    draw_response(
        canvas,
        Rect {
            x: 118,
            y: 344,
            w: 304,
            h: 36,
        },
        model.answer.as_str(),
    );
}

fn draw_runtime_panel(canvas: &mut Canvas, model: &HtmlUiModel) {
    draw_panel(
        canvas,
        Rect {
            x: 462,
            y: 192,
            w: 236,
            h: 86,
        },
        COLOR_SURFACE,
    );
    draw_text(canvas, 482, 216, "Runtime", COLOR_INK, 2);
    let mut x = 482usize;
    let mut y = 244usize;
    for pill in model.pills.iter().take(4) {
        let pill_w = text_width(pill.as_str(), 1).saturating_add(18).min(206);
        if x + pill_w > 680 {
            x = 482;
            y += 24;
        }
        draw_pill(canvas, x, y, pill.as_str(), COLOR_MUTED);
        x += pill_w + 8;
    }
}

fn draw_skills_panel(canvas: &mut Canvas, model: &HtmlUiModel) {
    draw_panel(
        canvas,
        Rect {
            x: 462,
            y: 296,
            w: 236,
            h: 102,
        },
        COLOR_SURFACE,
    );
    draw_text(canvas, 482, 320, "Skills", COLOR_INK, 2);
    let mut y = 348usize;
    for skill in model.skills.iter().take(3) {
        canvas.fill_rounded_rect(
            Rect {
                x: 482,
                y: y.saturating_sub(2),
                w: 196,
                h: 20,
            },
            5,
            COLOR_MUTED_SURFACE,
        );
        draw_text(canvas, 492, y + 2, skill.name.as_str(), COLOR_INK, 1);
        y += 22;
    }
}

fn draw_panel(canvas: &mut Canvas, rect: Rect, color: Rgb) {
    canvas.fill_rounded_rect(
        Rect {
            x: rect.x + 3,
            y: rect.y + 4,
            w: rect.w,
            h: rect.h,
        },
        9,
        COLOR_SHADOW,
    );
    canvas.fill_rounded_rect(rect, 8, color);
    canvas.stroke_rect(rect, COLOR_LINE);
}

fn draw_input(canvas: &mut Canvas, rect: Rect, text: &str) {
    canvas.fill_rounded_rect(rect, 6, COLOR_MUTED_SURFACE);
    canvas.stroke_rect(rect, COLOR_LINE);
    draw_text_wrapped(
        canvas,
        rect.x + 10,
        rect.y + 12,
        rect.w.saturating_sub(20),
        2,
        text,
        COLOR_INK,
        1,
    );
}

fn draw_response(canvas: &mut Canvas, rect: Rect, text: &str) {
    canvas.fill_rounded_rect(rect, 6, COLOR_DARK);
    draw_text_wrapped(
        canvas,
        rect.x + 10,
        rect.y + 10,
        rect.w.saturating_sub(20),
        1,
        text,
        COLOR_LIGHT_TEXT,
        1,
    );
}

fn draw_button(canvas: &mut Canvas, x: usize, y: usize, label: &str, primary: bool) {
    let color = if primary {
        COLOR_ACCENT
    } else {
        COLOR_MUTED_SURFACE
    };
    let ink = if primary { COLOR_LIGHT_TEXT } else { COLOR_INK };
    canvas.fill_rounded_rect(Rect { x, y, w: 96, h: 26 }, 6, color);
    canvas.stroke_rect(Rect { x, y, w: 96, h: 26 }, COLOR_LINE);
    draw_text(canvas, x + 12, y + 9, label, ink, 1);
}

fn draw_pill(canvas: &mut Canvas, x: usize, y: usize, label: &str, color: Rgb) {
    let width = text_width(label, 1).saturating_add(18).min(140);
    canvas.fill_rounded_rect(
        Rect {
            x,
            y,
            w: width,
            h: 22,
        },
        10,
        COLOR_MUTED_SURFACE,
    );
    canvas.stroke_rect(
        Rect {
            x,
            y,
            w: width,
            h: 22,
        },
        color,
    );
    draw_text(canvas, x + 9, y + 7, label, color, 1);
}

fn draw_text_wrapped(
    canvas: &mut Canvas,
    x: usize,
    y: usize,
    max_width: usize,
    max_lines: usize,
    text: &str,
    color: Rgb,
    scale: usize,
) {
    let clean = sanitize_ascii(text);
    let mut line = String::new();
    let mut line_index = 0usize;
    let char_limit = max_width / (6 * scale.max(1));
    for word in clean.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > char_limit {
            draw_text(
                canvas,
                x,
                y + line_index * (9 * scale.max(1) + 3),
                line.as_str(),
                color,
                scale,
            );
            line.clear();
            line_index += 1;
            if line_index >= max_lines {
                return;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() && line_index < max_lines {
        draw_text(
            canvas,
            x,
            y + line_index * (9 * scale.max(1) + 3),
            line.as_str(),
            color,
            scale,
        );
    }
}

fn draw_text(canvas: &mut Canvas, x: usize, y: usize, text: &str, color: Rgb, scale: usize) {
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

fn draw_glyph(canvas: &mut Canvas, x: usize, y: usize, byte: u8, color: Rgb, scale: usize) {
    let glyph = glyph_rows(byte);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5usize {
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
        b'_' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111,
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
        b'[' => [
            0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110,
        ],
        b']' => [
            0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110,
        ],
        b'(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        b')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        b'+' => [
            0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
        ],
        b'=' => [
            0b00000, 0b00000, 0b11111, 0b00000, 0b11111, 0b00000, 0b00000,
        ],
        b'<' => [
            0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010,
        ],
        b'>' => [
            0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000,
        ],
        b',' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100, 0b01000,
        ],
        _ => [
            0b11111, 0b10001, 0b00010, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
    }
}

fn text_width(text: &str, scale: usize) -> usize {
    sanitize_ascii(text).len().saturating_mul(6 * scale.max(1))
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

fn encode_ppm(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"P6\n");
    push_decimal_bytes(&mut out, width);
    out.push(b' ');
    push_decimal_bytes(&mut out, height);
    out.extend_from_slice(b"\n255\n");
    out.extend_from_slice(pixels);
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

fn render_ansi_preview(model: &HtmlUiModel) -> String {
    let mut out = String::new();
    push_ansi_header(&mut out, model.title.as_str());
    push_ansi_metrics(&mut out, &model.metrics);
    push_ansi_section(
        &mut out,
        "Prompt Composer",
        model.prompt.as_str(),
        COLOR_ACCENT,
    );
    push_ansi_buttons(&mut out, &model.buttons);
    push_ansi_section(&mut out, "Response", model.answer.as_str(), COLOR_DARK);
    push_ansi_skills(&mut out, &model.skills);
    out.push_str("\x1b[0m");
    out
}

fn push_ansi_header(out: &mut String, title: &str) {
    push_ansi_bar(out, COLOR_ACCENT_DARK);
    out.push_str("\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m ");
    push_fixed(out, title, ANSI_PREVIEW_COLS - 2);
    out.push_str(" \x1b[0m\n");
    out.push_str("\x1b[38;2;88;99;108m\x1b[48;2;255;255;255m ");
    push_fixed(out, "CPU-rendered native Hermes UI", ANSI_PREVIEW_COLS - 2);
    out.push_str(" \x1b[0m\n");
    push_ansi_bar(out, COLOR_LINE);
}

fn push_ansi_metrics(out: &mut String, metrics: &[MetricWidget]) {
    out.push_str("\x1b[38;2;23;32;38m\x1b[48;2;248;250;252m ");
    let mut line = String::new();
    for metric in metrics.iter().take(6) {
        if !line.is_empty() {
            line.push_str("  ");
        }
        line.push('[');
        line.push_str(metric.label.as_str());
        line.push(' ');
        line.push_str(metric.value.as_str());
        line.push(']');
    }
    push_fixed(out, line.as_str(), ANSI_PREVIEW_COLS - 2);
    out.push_str(" \x1b[0m\n");
}

fn push_ansi_section(out: &mut String, title: &str, body: &str, accent: Rgb) {
    push_ansi_bar(out, accent);
    out.push_str("\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m ");
    push_fixed(out, title, ANSI_PREVIEW_COLS - 2);
    out.push_str(" \x1b[0m\n");
    let clean = sanitize_ascii(body);
    push_ansi_wrapped(out, clean.as_str(), ANSI_PREVIEW_COLS - 4, 2);
}

fn push_ansi_buttons(out: &mut String, buttons: &[String]) {
    out.push_str("\x1b[38;2;234;242;248m\x1b[48;2;11;107;203m ");
    let mut line = String::new();
    for button in buttons.iter().take(2) {
        if !line.is_empty() {
            line.push_str("   ");
        }
        line.push_str("  ");
        line.push_str(button.as_str());
        line.push_str("  ");
    }
    push_fixed(out, line.as_str(), ANSI_PREVIEW_COLS - 2);
    out.push_str(" \x1b[0m\n");
}

fn push_ansi_skills(out: &mut String, skills: &[SkillWidget]) {
    push_ansi_bar(out, COLOR_OK);
    out.push_str("\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m ");
    push_fixed(out, "Skills", ANSI_PREVIEW_COLS - 2);
    out.push_str(" \x1b[0m\n");
    for skill in skills.iter().take(4) {
        let mut line = String::from("  ");
        line.push_str(skill.name.as_str());
        line.push_str(" - ");
        line.push_str(skill.description.as_str());
        out.push_str("\x1b[38;2;23;32;38m\x1b[48;2;248;250;252m ");
        push_fixed(out, line.as_str(), ANSI_PREVIEW_COLS - 2);
        out.push_str(" \x1b[0m\n");
    }
}

fn push_ansi_wrapped(out: &mut String, text: &str, width: usize, max_lines: usize) {
    let mut line = String::new();
    let mut lines = 0usize;
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            out.push_str("\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m  ");
            push_fixed(out, line.as_str(), width);
            out.push_str("  \x1b[0m\n");
            line.clear();
            lines += 1;
            if lines >= max_lines {
                return;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() && lines < max_lines {
        out.push_str("\x1b[38;2;23;32;38m\x1b[48;2;255;255;255m  ");
        push_fixed(out, line.as_str(), width);
        out.push_str("  \x1b[0m\n");
    }
}

fn push_ansi_bar(out: &mut String, color: Rgb) {
    out.push_str("\x1b[48;2;");
    append_usize_local(out, color.r as usize);
    out.push(';');
    append_usize_local(out, color.g as usize);
    out.push(';');
    append_usize_local(out, color.b as usize);
    out.push_str("m");
    for _ in 0..ANSI_PREVIEW_COLS {
        out.push(' ');
    }
    out.push_str("\x1b[0m\n");
}

fn push_fixed(out: &mut String, text: &str, width: usize) {
    let clean = sanitize_ascii(text);
    let mut written = 0usize;
    for byte in clean.bytes().take(width) {
        out.push(byte as char);
        written += 1;
    }
    for _ in written..width {
        out.push(' ');
    }
}

fn append_usize_local(out: &mut String, mut value: usize) {
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    if value == 0 {
        out.push('0');
        return;
    }
    while value > 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn extract_metrics(html: &str) -> Vec<MetricWidget> {
    let mut metrics = Vec::new();
    for block in extract_class_blocks(html, "metric") {
        let value = extract_tag(block.as_str(), "b").unwrap_or_else(|| String::from("?"));
        let label = extract_tag(block.as_str(), "span").unwrap_or_else(|| String::from("metric"));
        metrics.push(MetricWidget { label, value });
    }
    metrics
}

fn extract_skills(html: &str) -> Vec<SkillWidget> {
    let mut skills = Vec::new();
    for block in extract_class_blocks(html, "skill") {
        let name = extract_tag(block.as_str(), "b").unwrap_or_else(|| String::from("Skill"));
        let description = extract_tag(block.as_str(), "span").unwrap_or_else(String::new);
        let path = extract_tag(block.as_str(), "code").unwrap_or_else(String::new);
        skills.push(SkillWidget {
            name,
            description,
            path,
        });
    }
    skills
}

fn extract_class_texts(html: &str, class_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for block in extract_class_blocks(html, class_name) {
        out.push(html_text(block.as_str()));
    }
    out
}

fn extract_class_blocks(html: &str, class_name: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut pattern = String::from("class=\"");
    pattern.push_str(class_name);
    pattern.push('"');

    let mut search = 0usize;
    while search < html.len() {
        let Some(relative) = html[search..].find(pattern.as_str()) else {
            break;
        };
        let class_pos = search + relative;
        let start = find_previous_byte(html.as_bytes(), class_pos, b'<').unwrap_or(class_pos);
        let Some(open_end_relative) = html[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_relative + 1;
        let Some(close_relative) = html[open_end..].find("</div>") else {
            break;
        };
        let close = open_end + close_relative + "</div>".len();
        blocks.push(String::from(&html[start..close]));
        search = close;
    }
    blocks
}

fn extract_all_tags(html: &str, tag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut start_pattern = String::from("<");
    start_pattern.push_str(tag);
    let mut end_pattern = String::from("</");
    end_pattern.push_str(tag);
    end_pattern.push('>');

    let mut search = 0usize;
    while search < html.len() {
        let Some(open_relative) = html[search..].find(start_pattern.as_str()) else {
            break;
        };
        let open_start = search + open_relative;
        let Some(open_end_relative) = html[open_start..].find('>') else {
            break;
        };
        let content_start = open_start + open_end_relative + 1;
        let Some(close_relative) = html[content_start..].find(end_pattern.as_str()) else {
            break;
        };
        let close = content_start + close_relative;
        values.push(html_text(&html[content_start..close]));
        search = close + end_pattern.len();
    }
    values
}

fn extract_tag(html: &str, tag: &str) -> Option<String> {
    extract_all_tags(html, tag).into_iter().next()
}

fn html_text(input: &str) -> String {
    decode_entities(strip_tags(input).as_str())
}

fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for byte in input.bytes() {
        match byte {
            b'<' => in_tag = true,
            b'>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(byte as char),
            _ => {}
        }
    }
    collapse_spaces(out.as_str())
}

fn decode_entities(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"&lt;") {
            out.push('<');
            index += 4;
        } else if bytes[index..].starts_with(b"&gt;") {
            out.push('>');
            index += 4;
        } else if bytes[index..].starts_with(b"&amp;") {
            out.push('&');
            index += 5;
        } else if bytes[index..].starts_with(b"&quot;") {
            out.push('"');
            index += 6;
        } else if bytes[index..].starts_with(b"&#39;") {
            out.push('\'');
            index += 5;
        } else if bytes[index] == b'\n' || bytes[index] == b'\r' || bytes[index] == b'\t' {
            out.push(' ');
            index += 1;
        } else if (0x20..=0x7e).contains(&bytes[index]) {
            out.push(bytes[index] as char);
            index += 1;
        } else {
            out.push('.');
            index += 1;
        }
    }
    collapse_spaces(out.as_str())
}

fn collapse_spaces(input: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(byte as char);
            last_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn find_previous_byte(bytes: &[u8], mut pos: usize, target: u8) -> Option<usize> {
    while pos > 0 {
        pos -= 1;
        if bytes[pos] == target {
            return Some(pos);
        }
    }
    None
}

fn push_top(out: &mut String, width: usize, title: &str) {
    out.push('+');
    for _ in 0..width.saturating_sub(2) {
        out.push('-');
    }
    out.push_str("+\n");
    push_row(out, width, title);
}

fn push_rule(out: &mut String, width: usize) {
    out.push('+');
    for _ in 0..width.saturating_sub(2) {
        out.push('-');
    }
    out.push_str("+\n");
}

fn push_bottom(out: &mut String, width: usize) {
    push_rule(out, width);
}

fn push_row(out: &mut String, width: usize, text: &str) {
    let inner = width.saturating_sub(4);
    let clean = sanitize_ascii(text);
    out.push_str("| ");
    let mut written = 0usize;
    for byte in clean.bytes().take(inner) {
        out.push(byte as char);
        written += 1;
    }
    for _ in written..inner {
        out.push(' ');
    }
    out.push_str(" |\n");
}

fn push_wrapped(out: &mut String, width: usize, text: &str, indent: usize) {
    let inner = width.saturating_sub(4);
    let indent = core::cmp::min(indent, inner);
    let available = inner.saturating_sub(indent).max(1);
    let clean = sanitize_ascii(text);
    let mut line = String::new();
    for _ in 0..indent {
        line.push(' ');
    }

    for word in clean.split_whitespace() {
        if line.len() > indent && line.len() + 1 + word.len() > inner {
            push_row(out, width, line.as_str());
            line.clear();
            for _ in 0..indent {
                line.push(' ');
            }
        }

        if word.len() > available {
            if line.len() > indent {
                push_row(out, width, line.as_str());
                line.clear();
                for _ in 0..indent {
                    line.push(' ');
                }
            }
            let mut start = 0usize;
            while start < word.len() {
                let end = core::cmp::min(start + available, word.len());
                let mut chunk = String::new();
                for _ in 0..indent {
                    chunk.push(' ');
                }
                chunk.push_str(&word[start..end]);
                push_row(out, width, chunk.as_str());
                start = end;
            }
        } else {
            if line.len() > indent {
                line.push(' ');
            }
            line.push_str(word);
        }
    }

    if line.len() > indent {
        push_row(out, width, line.as_str());
    }
}

fn sanitize_ascii(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte == b'\n' || byte == b'\r' || byte == b'\t' {
            out.push(' ');
        } else if (0x20..=0x7e).contains(&byte) {
            out.push(byte as char);
        } else {
            out.push('.');
        }
    }
    out
}

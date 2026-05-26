//! Qt/QML vehicle cluster compatibility surface for SMROS.
//!
//! SMROS does not host a Qt runtime yet, so this module ports the dashboard as
//! a QML asset plus a native renderer that understands the cluster properties
//! and produces a CPU-rasterized instrument panel.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::user_level::fxfs;

const QML_CLUSTER_ROOT: &str = "/data/qml-cluster";
const QML_CLUSTER_QML_PATH: &str = "/data/qml-cluster/InstrumentCluster.qml";
const QML_CLUSTER_WINDOW_PATH: &str = "/data/qml-cluster/ClusterWindow.qml";
const QML_CLUSTER_PPM_PATH: &str = "/data/qml-cluster/cluster.ppm";

const CLUSTER_RENDER_WIDTH: usize = 960;
const CLUSTER_RENDER_HEIGHT: usize = 540;
const SPEED_MAX_KPH: usize = 240;
const RPM_MAX: usize = 8000;

const CLUSTER_QML_SOURCE: &str =
    include_str!("../../../host_shared/qml-cluster/InstrumentCluster.qml");
const CLUSTER_WINDOW_QML_SOURCE: &str =
    include_str!("../../../host_shared/qml-cluster/ClusterWindow.qml");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QmlClusterError {
    FxfsInit,
    FxfsPrepare,
    Qml,
    Render,
}

impl QmlClusterError {
    pub fn as_str(self) -> &'static str {
        match self {
            QmlClusterError::FxfsInit => "fxfs init",
            QmlClusterError::FxfsPrepare => "fxfs prepare",
            QmlClusterError::Qml => "qml",
            QmlClusterError::Render => "render",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QmlClusterState {
    pub title: String,
    pub width: usize,
    pub height: usize,
    pub speed_kph: usize,
    pub rpm: usize,
    pub battery_percent: usize,
    pub range_km: usize,
    pub gear: String,
    pub drive_mode: String,
    pub left_turn: bool,
    pub right_turn: bool,
    pub warning: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QmlClusterInfo {
    pub title: String,
    pub qml_path: &'static str,
    pub qml_bytes: usize,
    pub window_path: &'static str,
    pub window_bytes: usize,
    pub image_path: &'static str,
    pub image_bytes: usize,
    pub qml_width: usize,
    pub qml_height: usize,
    pub render_width: usize,
    pub render_height: usize,
    pub speed_kph: usize,
    pub rpm: usize,
    pub gear: String,
    pub drive_mode: String,
    pub backend: &'static str,
    pub qt_runtime: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QmlClusterRender {
    pub source_path: &'static str,
    pub image_path: &'static str,
    pub title: String,
    pub preview: String,
    pub width: usize,
    pub height: usize,
    pub image_bytes: usize,
    pub pixel_bytes: usize,
    pub widgets: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QmlClusterTestReport {
    pub qml_ok: bool,
    pub parse_ok: bool,
    pub render_ok: bool,
    pub fxfs_ok: bool,
    pub state: QmlClusterState,
    pub render: QmlClusterRender,
}

impl QmlClusterTestReport {
    pub fn passed(&self) -> bool {
        self.qml_ok && self.parse_ok && self.render_ok && self.fxfs_ok
    }
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

const COLOR_BG: Rgb = Rgb::new(7, 16, 19);
const COLOR_PANEL: Rgb = Rgb::new(18, 29, 32);
const COLOR_PANEL_SOFT: Rgb = Rgb::new(24, 39, 42);
const COLOR_LINE: Rgb = Rgb::new(54, 77, 82);
const COLOR_RING: Rgb = Rgb::new(45, 64, 69);
const COLOR_TEXT: Rgb = Rgb::new(244, 251, 248);
const COLOR_MUTED: Rgb = Rgb::new(137, 158, 163);
const COLOR_TEAL: Rgb = Rgb::new(0, 166, 166);
const COLOR_AMBER: Rgb = Rgb::new(255, 176, 0);
const COLOR_GREEN: Rgb = Rgb::new(72, 190, 123);
const COLOR_RED: Rgb = Rgb::new(226, 76, 76);
const COLOR_BLUE: Rgb = Rgb::new(95, 166, 230);
const COLOR_BLACK: Rgb = Rgb::new(3, 8, 10);

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

pub fn init() -> bool {
    prepare_storage().is_ok()
}

pub fn info() -> Result<QmlClusterInfo, QmlClusterError> {
    prepare_storage()?;
    let qml = read_text_file(QML_CLUSTER_QML_PATH)?;
    let state = parse_qml_state(qml.as_str())?;
    Ok(QmlClusterInfo {
        title: state.title,
        qml_path: QML_CLUSTER_QML_PATH,
        qml_bytes: qml.len(),
        window_path: QML_CLUSTER_WINDOW_PATH,
        window_bytes: fxfs::attrs(QML_CLUSTER_WINDOW_PATH)
            .map(|attrs| attrs.size)
            .unwrap_or(0),
        image_path: QML_CLUSTER_PPM_PATH,
        image_bytes: fxfs::attrs(QML_CLUSTER_PPM_PATH)
            .map(|attrs| attrs.size)
            .unwrap_or(0),
        qml_width: state.width,
        qml_height: state.height,
        render_width: CLUSTER_RENDER_WIDTH,
        render_height: CLUSTER_RENDER_HEIGHT,
        speed_kph: state.speed_kph,
        rpm: state.rpm,
        gear: state.gear,
        drive_mode: state.drive_mode,
        backend: "smros-qml-native",
        qt_runtime: "qml-asset-with-native-renderer",
    })
}

pub fn render_qml_source() -> Result<String, QmlClusterError> {
    prepare_storage()?;
    read_text_file(QML_CLUSTER_QML_PATH)
}

pub fn render_window_qml_source() -> Result<String, QmlClusterError> {
    prepare_storage()?;
    read_text_file(QML_CLUSTER_WINDOW_PATH)
}

pub fn render_cluster() -> Result<QmlClusterRender, QmlClusterError> {
    prepare_storage()?;
    let qml = read_text_file(QML_CLUSTER_QML_PATH)?;
    let state = parse_qml_state(qml.as_str())?;
    let mut canvas = Canvas::new(CLUSTER_RENDER_WIDTH, CLUSTER_RENDER_HEIGHT, COLOR_BG);
    draw_cluster(&mut canvas, &state);
    let preview = render_ansi_preview(&state);
    let ppm = encode_ppm(canvas.width, canvas.height, canvas.pixels.as_slice());
    fxfs::write_file(QML_CLUSTER_PPM_PATH, ppm.as_slice())
        .map_err(|_| QmlClusterError::FxfsPrepare)?;

    Ok(QmlClusterRender {
        source_path: QML_CLUSTER_QML_PATH,
        image_path: QML_CLUSTER_PPM_PATH,
        title: state.title,
        preview,
        width: canvas.width,
        height: canvas.height,
        image_bytes: ppm.len(),
        pixel_bytes: canvas.pixels.len(),
        widgets: 9,
    })
}

pub fn run_full_test() -> Result<QmlClusterTestReport, QmlClusterError> {
    prepare_storage()?;
    let qml = read_text_file(QML_CLUSTER_QML_PATH)?;
    let window_qml = read_text_file(QML_CLUSTER_WINDOW_PATH)?;
    let qml_ok = qml.contains("Item")
        && qml.contains("import QtQuick")
        && qml.contains("property int speedKph")
        && qml.contains("property string gear")
        && qml.contains("Canvas")
        && window_qml.contains("Window")
        && window_qml.contains("import QtQuick.Window")
        && window_qml.contains("InstrumentCluster");
    if !qml_ok {
        return Err(QmlClusterError::Qml);
    }

    let state = parse_qml_state(qml.as_str())?;
    let parse_ok = state.width == 1280
        && state.height == 720
        && state.speed_kph == 88
        && state.rpm == 3200
        && state.battery_percent == 78
        && state.range_km == 326
        && state.gear == "D"
        && state.drive_mode == "Comfort"
        && state.warning == "ADAS ready";
    if !parse_ok {
        return Err(QmlClusterError::Qml);
    }

    let render = render_cluster()?;
    let render_ok = render.width == CLUSTER_RENDER_WIDTH
        && render.height == CLUSTER_RENDER_HEIGHT
        && render.image_bytes > render.pixel_bytes
        && render.preview.contains("SMROS Qt/QML Vehicle Cluster")
        && render.preview.contains("Speed 88 km/h");
    if !render_ok {
        return Err(QmlClusterError::Render);
    }

    let fxfs_ok = fxfs::attrs(QML_CLUSTER_QML_PATH).is_ok()
        && fxfs::attrs(QML_CLUSTER_PPM_PATH)
            .map(|attrs| attrs.size == render.image_bytes)
            .unwrap_or(false);
    if !fxfs_ok {
        return Err(QmlClusterError::FxfsPrepare);
    }

    Ok(QmlClusterTestReport {
        qml_ok,
        parse_ok,
        render_ok,
        fxfs_ok,
        state,
        render,
    })
}

pub fn smoke_test() -> bool {
    run_full_test()
        .map(|report| report.passed())
        .unwrap_or(false)
}

fn prepare_storage() -> Result<(), QmlClusterError> {
    if !fxfs::init() {
        return Err(QmlClusterError::FxfsInit);
    }
    create_dir("/data")?;
    create_dir(QML_CLUSTER_ROOT)?;
    ensure_exact_file(QML_CLUSTER_QML_PATH, CLUSTER_QML_SOURCE)?;
    ensure_exact_file(QML_CLUSTER_WINDOW_PATH, CLUSTER_WINDOW_QML_SOURCE)?;
    Ok(())
}

fn create_dir(path: &str) -> Result<(), QmlClusterError> {
    fxfs::create_dir(path)
        .map(|_| ())
        .map_err(|_| QmlClusterError::FxfsPrepare)
}

fn ensure_exact_file(path: &str, data: &str) -> Result<(), QmlClusterError> {
    if let Ok(current) = read_text_file(path) {
        if current == data {
            return Ok(());
        }
    }
    fxfs::write_file(path, data.as_bytes())
        .map(|_| ())
        .map_err(|_| QmlClusterError::FxfsPrepare)
}

fn read_text_file(path: &str) -> Result<String, QmlClusterError> {
    let attrs = fxfs::attrs(path).map_err(|_| QmlClusterError::FxfsPrepare)?;
    let mut out = Vec::new();
    out.resize(attrs.size, 0);
    let read = fxfs::read_file(path, &mut out).map_err(|_| QmlClusterError::FxfsPrepare)?;
    out.truncate(read);
    String::from_utf8(out).map_err(|_| QmlClusterError::FxfsPrepare)
}

fn parse_qml_state(qml: &str) -> Result<QmlClusterState, QmlClusterError> {
    if qml.trim().is_empty() || !qml.contains("Item") || !qml.contains("import QtQuick") {
        return Err(QmlClusterError::Qml);
    }

    Ok(QmlClusterState {
        title: extract_string_property(qml, "title").ok_or(QmlClusterError::Qml)?,
        width: extract_direct_usize(qml, "width").ok_or(QmlClusterError::Qml)?,
        height: extract_direct_usize(qml, "height").ok_or(QmlClusterError::Qml)?,
        speed_kph: extract_int_property(qml, "speedKph").ok_or(QmlClusterError::Qml)?,
        rpm: extract_int_property(qml, "rpm").ok_or(QmlClusterError::Qml)?,
        battery_percent: extract_int_property(qml, "batteryPercent").ok_or(QmlClusterError::Qml)?,
        range_km: extract_int_property(qml, "rangeKm").ok_or(QmlClusterError::Qml)?,
        gear: extract_string_property(qml, "gear").ok_or(QmlClusterError::Qml)?,
        drive_mode: extract_string_property(qml, "driveMode").ok_or(QmlClusterError::Qml)?,
        left_turn: extract_bool_property(qml, "leftTurn").ok_or(QmlClusterError::Qml)?,
        right_turn: extract_bool_property(qml, "rightTurn").ok_or(QmlClusterError::Qml)?,
        warning: extract_string_property(qml, "warning").ok_or(QmlClusterError::Qml)?,
    })
}

fn extract_direct_usize(qml: &str, key: &str) -> Option<usize> {
    for line in qml.lines() {
        let trimmed = trim_ascii(line);
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = trim_ascii(rest);
            if let Some(value) = rest.strip_prefix(':') {
                return parse_usize_prefix(trim_ascii(value));
            }
        }
    }
    None
}

fn extract_direct_string(qml: &str, key: &str) -> Option<String> {
    for line in qml.lines() {
        let trimmed = trim_ascii(line);
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = trim_ascii(rest);
            if let Some(value) = rest.strip_prefix(':') {
                return parse_quoted(trim_ascii(value));
            }
        }
    }
    None
}

fn extract_int_property(qml: &str, name: &str) -> Option<usize> {
    extract_property_value(qml, "property int ", name).and_then(parse_usize_prefix)
}

fn extract_string_property(qml: &str, name: &str) -> Option<String> {
    extract_property_value(qml, "property string ", name).and_then(parse_quoted)
}

fn extract_bool_property(qml: &str, name: &str) -> Option<bool> {
    let value = extract_property_value(qml, "property bool ", name)?;
    if value.starts_with("true") {
        Some(true)
    } else if value.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_property_value<'a>(qml: &'a str, prefix: &str, name: &str) -> Option<&'a str> {
    for line in qml.lines() {
        let trimmed = trim_ascii(line);
        let Some(rest) = trimmed.strip_prefix(prefix) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(name) else {
            continue;
        };
        let rest = trim_ascii(rest);
        let Some(value) = rest.strip_prefix(':') else {
            continue;
        };
        return Some(trim_ascii(value));
    }
    None
}

fn parse_usize_prefix(value: &str) -> Option<usize> {
    let mut out = 0usize;
    let mut saw_digit = false;
    for byte in value.bytes() {
        if byte.is_ascii_digit() {
            saw_digit = true;
            out = out.checked_mul(10)?;
            out = out.checked_add((byte - b'0') as usize)?;
        } else {
            break;
        }
    }
    if saw_digit {
        Some(out)
    } else {
        None
    }
}

fn parse_quoted(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.first().copied() != Some(b'"') {
        return None;
    }
    let mut out = String::new();
    for byte in bytes.iter().copied().skip(1) {
        if byte == b'"' {
            return Some(out);
        }
        if byte == b'\n' || byte == b'\r' {
            out.push(' ');
        } else if (0x20..=0x7e).contains(&byte) {
            out.push(byte as char);
        }
    }
    None
}

fn trim_ascii(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &value[start..end]
}

fn draw_cluster(canvas: &mut Canvas, state: &QmlClusterState) {
    draw_background(canvas);
    draw_top_status(canvas, state);
    draw_gauge(
        canvas,
        245,
        304,
        176,
        state.speed_kph,
        SPEED_MAX_KPH,
        "SPEED",
        "KM/H",
        COLOR_TEAL,
    );
    draw_gauge(
        canvas,
        715,
        304,
        176,
        state.rpm,
        RPM_MAX,
        "POWER",
        "RPM",
        COLOR_AMBER,
    );
    draw_vehicle(canvas, state);
    draw_bottom_status(canvas, state);
}

fn draw_background(canvas: &mut Canvas) {
    canvas.fill_rect(
        Rect {
            x: 0,
            y: 0,
            w: canvas.width,
            h: canvas.height,
        },
        COLOR_BG,
    );
    let mut x = 0usize;
    while x < canvas.width {
        canvas.fill_rect(
            Rect {
                x,
                y: 0,
                w: 1,
                h: canvas.height,
            },
            Rgb::new(10, 25, 29),
        );
        x += 80;
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
            Rgb::new(10, 25, 29),
        );
        y += 60;
    }
}

fn draw_top_status(canvas: &mut Canvas, state: &QmlClusterState) {
    draw_panel(
        canvas,
        Rect {
            x: 28,
            y: 22,
            w: 904,
            h: 76,
        },
    );
    draw_text(canvas, 50, 46, state.title.as_str(), COLOR_TEXT, 3);
    draw_text(
        canvas,
        50,
        76,
        "QML ASSET / NATIVE SMROS RENDERER",
        COLOR_MUTED,
        1,
    );
    draw_turn(canvas, 398, 46, "<<", state.left_turn);
    draw_text_centered(canvas, 480, 42, state.gear.as_str(), COLOR_AMBER, 5);
    draw_turn(canvas, 532, 46, ">>", state.right_turn);
    draw_text_centered(canvas, 480, 84, state.drive_mode.as_str(), COLOR_BLUE, 2);
    draw_text(canvas, 720, 48, state.warning.as_str(), COLOR_TEXT, 2);
}

fn draw_turn(canvas: &mut Canvas, x: usize, y: usize, label: &str, active: bool) {
    let color = if active { COLOR_GREEN } else { COLOR_RING };
    canvas.fill_rounded_rect(Rect { x, y, w: 46, h: 30 }, 6, COLOR_PANEL_SOFT);
    canvas.stroke_rect(Rect { x, y, w: 46, h: 30 }, color);
    draw_text(canvas, x + 10, y + 9, label, color, 1);
}

fn draw_gauge(
    canvas: &mut Canvas,
    cx: usize,
    cy: usize,
    radius: usize,
    value: usize,
    max: usize,
    label: &str,
    unit: &str,
    accent: Rgb,
) {
    draw_panel(
        canvas,
        Rect {
            x: cx.saturating_sub(208),
            y: cy.saturating_sub(202),
            w: 416,
            h: 396,
        },
    );
    draw_ring(canvas, cx, cy, radius - 8, radius + 8, COLOR_RING);
    draw_arc_progress(canvas, cx, cy, radius, value.min(max), max, accent);
    draw_ticks(canvas, cx, cy, radius);

    let needle = arc_point(cx, cy, radius.saturating_sub(48), value.min(max), max);
    draw_thick_line(
        canvas,
        cx as isize,
        cy as isize,
        needle.0,
        needle.1,
        COLOR_TEXT,
        2,
    );
    fill_circle(canvas, cx as isize, cy as isize, 10, accent);
    fill_circle(canvas, cx as isize, cy as isize, 4, COLOR_BLACK);

    let mut value_text = String::new();
    append_usize(&mut value_text, value, 0);
    draw_text_centered(
        canvas,
        cx,
        cy.saturating_sub(28),
        value_text.as_str(),
        COLOR_TEXT,
        5,
    );
    draw_text_centered(canvas, cx, cy + 28, unit, COLOR_MUTED, 2);
    draw_text_centered(canvas, cx, cy + 78, label, accent, 2);
}

fn draw_ticks(canvas: &mut Canvas, cx: usize, cy: usize, radius: usize) {
    let mut index = 0usize;
    while index <= 8 {
        let value = index * 1000;
        let outer = arc_point(cx, cy, radius + 18, value, 8000);
        let inner = arc_point(cx, cy, radius + 4, value, 8000);
        draw_thick_line(canvas, inner.0, inner.1, outer.0, outer.1, COLOR_LINE, 1);
        index += 1;
    }
}

fn draw_arc_progress(
    canvas: &mut Canvas,
    cx: usize,
    cy: usize,
    radius: usize,
    value: usize,
    max: usize,
    color: Rgb,
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
        draw_thick_line(canvas, previous.0, previous.1, point.0, point.1, color, 5);
        previous = point;
        step += 1;
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

fn draw_vehicle(canvas: &mut Canvas, state: &QmlClusterState) {
    draw_panel(
        canvas,
        Rect {
            x: 392,
            y: 154,
            w: 176,
            h: 268,
        },
    );
    draw_text_centered(canvas, 480, 180, "LANE", COLOR_MUTED, 2);
    draw_line(canvas, 428, 224, 390, 390, COLOR_BLUE);
    draw_line(canvas, 532, 224, 570, 390, COLOR_BLUE);
    draw_thick_line(canvas, 448, 396, 428, 458, COLOR_LINE, 1);
    draw_thick_line(canvas, 512, 396, 532, 458, COLOR_LINE, 1);
    canvas.fill_rounded_rect(
        Rect {
            x: 440,
            y: 238,
            w: 80,
            h: 136,
        },
        18,
        COLOR_TEAL,
    );
    canvas.fill_rounded_rect(
        Rect {
            x: 454,
            y: 254,
            w: 52,
            h: 44,
        },
        8,
        COLOR_BLACK,
    );
    canvas.fill_rounded_rect(
        Rect {
            x: 454,
            y: 310,
            w: 52,
            h: 42,
        },
        8,
        Rgb::new(13, 45, 48),
    );
    fill_circle(canvas, 442, 356, 8, COLOR_BLACK);
    fill_circle(canvas, 518, 356, 8, COLOR_BLACK);
    draw_text_centered(canvas, 480, 402, state.warning.as_str(), COLOR_GREEN, 1);
}

fn draw_bottom_status(canvas: &mut Canvas, state: &QmlClusterState) {
    draw_panel(
        canvas,
        Rect {
            x: 50,
            y: 446,
            w: 390,
            h: 68,
        },
    );
    draw_panel(
        canvas,
        Rect {
            x: 520,
            y: 446,
            w: 390,
            h: 68,
        },
    );
    let mut range = String::from("RANGE ");
    append_usize(&mut range, state.range_km, 0);
    range.push_str(" KM");
    draw_text(canvas, 76, 470, range.as_str(), COLOR_GREEN, 2);
    draw_bar(
        canvas,
        250,
        472,
        154,
        18,
        state.range_km.min(420),
        420,
        COLOR_GREEN,
    );

    let mut battery = String::from("BATTERY ");
    append_usize(&mut battery, state.battery_percent, 0);
    battery.push('%');
    draw_text(canvas, 546, 470, battery.as_str(), COLOR_GREEN, 2);
    draw_bar(
        canvas,
        748,
        472,
        128,
        18,
        state.battery_percent.min(100),
        100,
        COLOR_GREEN,
    );
}

fn draw_bar(
    canvas: &mut Canvas,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    value: usize,
    max: usize,
    color: Rgb,
) {
    canvas.fill_rounded_rect(Rect { x, y, w, h }, 5, COLOR_BLACK);
    canvas.stroke_rect(Rect { x, y, w, h }, COLOR_LINE);
    if max == 0 || w <= 6 || h <= 6 {
        return;
    }
    let fill = value.min(max).saturating_mul(w.saturating_sub(6)) / max;
    canvas.fill_rounded_rect(
        Rect {
            x: x + 3,
            y: y + 3,
            w: fill,
            h: h - 6,
        },
        3,
        color,
    );
}

fn draw_panel(canvas: &mut Canvas, rect: Rect) {
    canvas.fill_rounded_rect(rect, 10, COLOR_PANEL);
    canvas.stroke_rect(rect, COLOR_LINE);
}

fn draw_ring(canvas: &mut Canvas, cx: usize, cy: usize, inner: usize, outer: usize, color: Rgb) {
    let outer_sq = outer.saturating_mul(outer) as isize;
    let inner_sq = inner.saturating_mul(inner) as isize;
    let x0 = cx.saturating_sub(outer).min(canvas.width);
    let x1 = cx.saturating_add(outer).min(canvas.width.saturating_sub(1));
    let y0 = cy.saturating_sub(outer).min(canvas.height);
    let y1 = cy
        .saturating_add(outer)
        .min(canvas.height.saturating_sub(1));
    let mut y = y0;
    while y <= y1 {
        let mut x = x0;
        while x <= x1 {
            let dx = x as isize - cx as isize;
            let dy = y as isize - cy as isize;
            let dist = dx * dx + dy * dy;
            if dist >= inner_sq && dist <= outer_sq {
                canvas.set_pixel(x, y, color);
            }
            x += 1;
        }
        y += 1;
    }
}

fn fill_circle(canvas: &mut Canvas, cx: isize, cy: isize, radius: usize, color: Rgb) {
    let r = radius as isize;
    let r_sq = r * r;
    let mut y = cy - r;
    while y <= cy + r {
        let mut x = cx - r;
        while x <= cx + r {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r_sq && x >= 0 && y >= 0 {
                canvas.set_pixel(x as usize, y as usize, color);
            }
            x += 1;
        }
        y += 1;
    }
}

fn draw_line(canvas: &mut Canvas, x0: usize, y0: usize, x1: usize, y1: usize, color: Rgb) {
    draw_line_signed(
        canvas,
        x0 as isize,
        y0 as isize,
        x1 as isize,
        y1 as isize,
        color,
    );
}

fn draw_thick_line(
    canvas: &mut Canvas,
    x0: isize,
    y0: isize,
    x1: isize,
    y1: isize,
    color: Rgb,
    radius: isize,
) {
    let mut oy = -radius;
    while oy <= radius {
        let mut ox = -radius;
        while ox <= radius {
            if ox * ox + oy * oy <= radius * radius {
                draw_line_signed(canvas, x0 + ox, y0 + oy, x1 + ox, y1 + oy, color);
            }
            ox += 1;
        }
        oy += 1;
    }
}

fn draw_line_signed(
    canvas: &mut Canvas,
    mut x0: isize,
    mut y0: isize,
    x1: isize,
    y1: isize,
    color: Rgb,
) {
    let dx = abs_isize(x1 - x0);
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -abs_isize(y1 - y0);
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if x0 >= 0 && y0 >= 0 {
            canvas.set_pixel(x0 as usize, y0 as usize, color);
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

fn abs_isize(value: isize) -> isize {
    if value < 0 {
        -value
    } else {
        value
    }
}

fn draw_text_centered(
    canvas: &mut Canvas,
    cx: usize,
    y: usize,
    text: &str,
    color: Rgb,
    scale: usize,
) {
    let width = text_width(text, scale);
    let x = cx.saturating_sub(width / 2);
    draw_text(canvas, x, y, text, color, scale);
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

fn render_ansi_preview(state: &QmlClusterState) -> String {
    let mut out = String::new();
    push_ansi_block(&mut out, COLOR_BG);
    out.push('\n');
    push_ansi_text_line(
        &mut out,
        "  SMROS Qt/QML Vehicle Cluster",
        COLOR_TEXT,
        COLOR_BG,
        84,
    );
    let mut top = String::from("  ");
    top.push_str(if state.left_turn { "<<" } else { "< " });
    top.push_str("        Gear ");
    top.push_str(state.gear.as_str());
    top.push_str("    Mode ");
    top.push_str(state.drive_mode.as_str());
    top.push_str("        ");
    top.push_str(if state.right_turn { ">>" } else { " >" });
    push_ansi_text_line(&mut out, top.as_str(), COLOR_AMBER, COLOR_BG, 84);

    push_ansi_text_line(&mut out, "  +----------------------------+    +------------------+    +----------------------------+", COLOR_LINE, COLOR_BG, 84);
    push_ansi_text_line(&mut out, "  |                            |    |                  |    |                            |", COLOR_LINE, COLOR_BG, 84);

    let mut gauges = String::from("  | Speed ");
    append_usize(&mut gauges, state.speed_kph, 0);
    gauges.push_str(" km/h");
    push_spaces_to(&mut gauges, 31);
    gauges.push_str("|    |      LANE        |    | RPM ");
    append_usize(&mut gauges, state.rpm, 0);
    push_spaces_to(&mut gauges, 78);
    gauges.push('|');
    push_ansi_text_line(&mut out, gauges.as_str(), COLOR_TEXT, COLOR_BG, 84);

    let speed_fill = state.speed_kph.min(SPEED_MAX_KPH) * 20 / SPEED_MAX_KPH;
    let rpm_fill = state.rpm.min(RPM_MAX) * 20 / RPM_MAX;
    let mut bars = String::from("  | ");
    push_meter(&mut bars, speed_fill, 20);
    bars.push_str("       |    |     /----\\       |    | ");
    push_meter(&mut bars, rpm_fill, 20);
    bars.push_str("       |");
    push_ansi_text_line(&mut out, bars.as_str(), COLOR_TEAL, COLOR_BG, 84);

    push_ansi_text_line(&mut out, "  |                            |    |    | SMROS |      |    |                            |", COLOR_LINE, COLOR_BG, 84);
    push_ansi_text_line(&mut out, "  |        native PPM          |    |     \\____/       |    |        Qt window QML       |", COLOR_MUTED, COLOR_BG, 84);
    push_ansi_text_line(&mut out, "  +----------------------------+    +------------------+    +----------------------------+", COLOR_LINE, COLOR_BG, 84);

    let mut bottom = String::from("  Range ");
    append_usize(&mut bottom, state.range_km, 0);
    bottom.push_str(" km  ");
    let range_fill = state.range_km.min(420) * 18 / 420;
    push_meter(&mut bottom, range_fill, 18);
    bottom.push_str("     Battery ");
    append_usize(&mut bottom, state.battery_percent, 0);
    bottom.push_str("% ");
    let battery_fill = state.battery_percent.min(100) * 18 / 100;
    push_meter(&mut bottom, battery_fill, 18);
    push_ansi_text_line(&mut out, bottom.as_str(), COLOR_GREEN, COLOR_BG, 84);

    let mut warn = String::from("  Warning ");
    warn.push_str(state.warning.as_str());
    warn.push_str("  Source ");
    warn.push_str(QML_CLUSTER_QML_PATH);
    push_ansi_text_line(&mut out, warn.as_str(), COLOR_TEXT, COLOR_BG, 84);
    out.push_str("\x1b[0m");
    out
}

fn push_meter(out: &mut String, filled: usize, width: usize) {
    out.push('[');
    let mut index = 0usize;
    while index < width {
        out.push(if index < filled { '#' } else { '-' });
        index += 1;
    }
    out.push(']');
}

fn push_spaces_to(out: &mut String, target_len: usize) {
    while out.len() < target_len {
        out.push(' ');
    }
}

fn push_ansi_block(out: &mut String, color: Rgb) {
    out.push_str("\x1b[48;2;");
    append_usize(out, color.r as usize, 0);
    out.push(';');
    append_usize(out, color.g as usize, 0);
    out.push(';');
    append_usize(out, color.b as usize, 0);
    out.push_str("m");
    let mut index = 0usize;
    while index < 84 {
        out.push(' ');
        index += 1;
    }
    out.push_str("\x1b[0m");
}

fn push_ansi_text_line(out: &mut String, text: &str, fg: Rgb, bg: Rgb, width: usize) {
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
    out.push_str("m");
    push_fixed_ascii(out, text, width);
    out.push_str("\x1b[0m\n");
}

fn push_fixed_ascii(out: &mut String, text: &str, width: usize) {
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

fn append_usize(out: &mut String, mut value: usize, min_width: usize) {
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

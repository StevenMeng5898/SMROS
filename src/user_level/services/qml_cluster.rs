//! Qt/QML vehicle cluster compatibility surface for SMROS.
//!
//! SMROS does not host a Qt runtime yet, so this module stores the QML assets,
//! parses their dashboard properties, and mirrors the UI through the SMROS
//! LVGL-style renderer.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::user_level::fxfs;
use crate::user_level::lvgl;

const QML_CLUSTER_ROOT: &str = "/data/qml-cluster";
const QML_CLUSTER_QML_PATH: &str = "/data/qml-cluster/InstrumentCluster.qml";
const QML_CLUSTER_WINDOW_PATH: &str = "/data/qml-cluster/ClusterWindow.qml";
const QML_CLUSTER_PPM_PATH: &str = "/data/qml-cluster/cluster.ppm";

const CLUSTER_RENDER_WIDTH: usize = 480;
const CLUSTER_RENDER_HEIGHT: usize = 270;
const CLUSTER_DESIGN_WIDTH: usize = 960;
const CLUSTER_DESIGN_HEIGHT: usize = 540;
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
    pub lvgl_port: &'static str,
    pub display_backend: &'static str,
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
    pub renderer: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QmlClusterTestReport {
    pub qml_ok: bool,
    pub parse_ok: bool,
    pub render_ok: bool,
    pub fxfs_ok: bool,
    pub lvgl_ok: bool,
    pub state: QmlClusterState,
    pub render: QmlClusterRender,
}

impl QmlClusterTestReport {
    pub fn passed(&self) -> bool {
        self.qml_ok && self.parse_ok && self.render_ok && self.fxfs_ok && self.lvgl_ok
    }
}

pub fn init() -> bool {
    prepare_storage().is_ok()
}

pub fn info() -> Result<QmlClusterInfo, QmlClusterError> {
    prepare_storage()?;
    let qml = read_text_file(QML_CLUSTER_QML_PATH)?;
    let state = parse_qml_state(qml.as_str())?;
    let lvgl_info = lvgl::info();
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
        backend: "smros-qml-lvgl",
        qt_runtime: "qml-asset-with-lvgl-renderer",
        lvgl_port: lvgl_info.name,
        display_backend: lvgl_info.display_backend,
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
    let mut canvas = lvgl::Canvas::try_new(
        CLUSTER_RENDER_WIDTH,
        CLUSTER_RENDER_HEIGHT,
        lvgl::AUTOMOTIVE_THEME.bg,
    )
    .map_err(|_| QmlClusterError::Render)?;
    draw_cluster(&mut canvas, &state);
    let preview = render_ansi_preview(&state);
    let ppm = lvgl::try_encode_ppm(canvas.width, canvas.height, canvas.pixels.as_slice())
        .map_err(|_| QmlClusterError::Render)?;
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
        widgets: 12,
        renderer: lvgl::LVGL_PORT_NAME,
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
        && render.preview.contains("LVGL")
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

    let lvgl_ok = lvgl::info().name == render.renderer && lvgl::init();
    if !lvgl_ok {
        return Err(QmlClusterError::Render);
    }

    Ok(QmlClusterTestReport {
        qml_ok,
        parse_ok,
        render_ok,
        fxfs_ok,
        lvgl_ok,
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
    lvgl::init();
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

fn draw_cluster(canvas: &mut lvgl::Canvas, state: &QmlClusterState) {
    let theme = lvgl::AUTOMOTIVE_THEME;
    lvgl::draw_background_grid(canvas, theme, sx(80), sy(60));
    draw_top_status(canvas, state);
    lvgl::draw_meter_card(
        canvas,
        lvgl::MeterSpec {
            cx: sx(245),
            cy: sy(304),
            radius: sr(176),
            value: state.speed_kph,
            max: SPEED_MAX_KPH,
            label: "SPEED",
            unit: "KM/H",
            accent: theme.accent,
        },
        theme,
    );
    lvgl::draw_meter_card(
        canvas,
        lvgl::MeterSpec {
            cx: sx(715),
            cy: sy(304),
            radius: sr(176),
            value: state.rpm,
            max: RPM_MAX,
            label: "POWER",
            unit: "RPM",
            accent: theme.warn,
        },
        theme,
    );
    draw_vehicle(canvas, state);
    draw_bottom_status(canvas, state);
}

fn draw_top_status(canvas: &mut lvgl::Canvas, state: &QmlClusterState) {
    let theme = lvgl::AUTOMOTIVE_THEME;
    lvgl::draw_header(
        canvas,
        lvgl::Rect {
            x: sx(28),
            y: sy(22),
            w: sx(904),
            h: sy(76),
        },
        state.title.as_str(),
        "QML ASSET / SMROS LVGL RENDERER",
        theme,
    );
    draw_turn(canvas, sx(398), sy(46), "<<", state.left_turn);
    lvgl::draw_label_centered(canvas, sx(480), sy(42), state.gear.as_str(), theme.warn, 2);
    draw_turn(canvas, sx(532), sy(46), ">>", state.right_turn);
    lvgl::draw_label_centered(
        canvas,
        sx(480),
        sy(84),
        state.drive_mode.as_str(),
        theme.accent_2,
        1,
    );
    lvgl::draw_label(
        canvas,
        sx(720),
        sy(48),
        state.warning.as_str(),
        theme.text,
        1,
    );
}

fn draw_turn(canvas: &mut lvgl::Canvas, x: usize, y: usize, label: &str, active: bool) {
    let theme = lvgl::AUTOMOTIVE_THEME;
    let color = if active { theme.ok } else { theme.line };
    canvas.fill_rounded_rect(
        lvgl::Rect {
            x,
            y,
            w: sx(46),
            h: sy(30),
        },
        sr(6),
        theme.surface_alt,
    );
    canvas.stroke_rect(
        lvgl::Rect {
            x,
            y,
            w: sx(46),
            h: sy(30),
        },
        color,
    );
    lvgl::draw_label(canvas, x + sx(10), y + sy(9), label, color, 1);
}

fn draw_vehicle(canvas: &mut lvgl::Canvas, state: &QmlClusterState) {
    let theme = lvgl::AUTOMOTIVE_THEME;
    lvgl::draw_panel(
        canvas,
        lvgl::Rect {
            x: sx(392),
            y: sy(154),
            w: sx(176),
            h: sy(268),
        },
        sr(12),
        theme,
    );
    lvgl::draw_label_centered(canvas, sx(480), sy(180), "LANE", theme.muted, 1);
    canvas.draw_line(sx(428), sy(224), sx(390), sy(390), theme.accent_2);
    canvas.draw_line(sx(532), sy(224), sx(570), sy(390), theme.accent_2);
    canvas.draw_thick_line(
        sx(448) as isize,
        sy(396) as isize,
        sx(428) as isize,
        sy(458) as isize,
        theme.line,
        1,
    );
    canvas.draw_thick_line(
        sx(512) as isize,
        sy(396) as isize,
        sx(532) as isize,
        sy(458) as isize,
        theme.line,
        1,
    );
    canvas.fill_rounded_rect(
        lvgl::Rect {
            x: sx(440),
            y: sy(238),
            w: sx(80),
            h: sy(136),
        },
        sr(18),
        theme.accent,
    );
    canvas.fill_rounded_rect(
        lvgl::Rect {
            x: sx(454),
            y: sy(254),
            w: sx(52),
            h: sy(44),
        },
        sr(8),
        theme.shadow,
    );
    canvas.fill_rounded_rect(
        lvgl::Rect {
            x: sx(454),
            y: sy(310),
            w: sx(52),
            h: sy(42),
        },
        sr(8),
        lvgl::Color::new(13, 45, 48),
    );
    canvas.fill_circle(sx(442) as isize, sy(356) as isize, sr(8), theme.shadow);
    canvas.fill_circle(sx(518) as isize, sy(356) as isize, sr(8), theme.shadow);
    lvgl::draw_label_centered(
        canvas,
        sx(480),
        sy(402),
        state.warning.as_str(),
        theme.ok,
        1,
    );
}

fn draw_bottom_status(canvas: &mut lvgl::Canvas, state: &QmlClusterState) {
    let theme = lvgl::AUTOMOTIVE_THEME;
    lvgl::draw_panel(
        canvas,
        lvgl::Rect {
            x: sx(50),
            y: sy(446),
            w: sx(390),
            h: sy(68),
        },
        sr(10),
        theme,
    );
    lvgl::draw_panel(
        canvas,
        lvgl::Rect {
            x: sx(520),
            y: sy(446),
            w: sx(390),
            h: sy(68),
        },
        sr(10),
        theme,
    );
    let mut range = String::from("RANGE ");
    lvgl::append_usize(&mut range, state.range_km, 0);
    range.push_str(" KM");
    lvgl::draw_label(canvas, sx(76), sy(470), range.as_str(), theme.ok, 1);
    lvgl::draw_progress_bar(
        canvas,
        lvgl::Rect {
            x: sx(250),
            y: sy(472),
            w: sx(154),
            h: sy(18),
        },
        state.range_km.min(420),
        420,
        theme.ok,
        theme,
    );

    let mut battery = String::from("BATTERY ");
    lvgl::append_usize(&mut battery, state.battery_percent, 0);
    battery.push('%');
    lvgl::draw_label(canvas, sx(546), sy(470), battery.as_str(), theme.ok, 1);
    lvgl::draw_progress_bar(
        canvas,
        lvgl::Rect {
            x: sx(748),
            y: sy(472),
            w: sx(128),
            h: sy(18),
        },
        state.battery_percent.min(100),
        100,
        theme.ok,
        theme,
    );
}

fn render_ansi_preview(state: &QmlClusterState) -> String {
    let theme = lvgl::AUTOMOTIVE_THEME;
    let mut out = String::new();
    lvgl::push_ansi_line(
        &mut out,
        "  SMROS Qt/QML Vehicle Cluster  |  LVGL renderer",
        theme.text,
        theme.bg,
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
    lvgl::push_ansi_line(&mut out, top.as_str(), theme.warn, theme.bg, 84);

    lvgl::push_ansi_line(
        &mut out,
        "  +----------------------------+    +------------------+    +----------------------------+",
        theme.line,
        theme.bg,
        84,
    );
    lvgl::push_ansi_line(
        &mut out,
        "  |                            |    |                  |    |                            |",
        theme.line,
        theme.bg,
        84,
    );

    let mut gauges = String::from("  | Speed ");
    lvgl::append_usize(&mut gauges, state.speed_kph, 0);
    gauges.push_str(" km/h");
    push_spaces_to(&mut gauges, 31);
    gauges.push_str("|    |      LANE        |    | RPM ");
    lvgl::append_usize(&mut gauges, state.rpm, 0);
    push_spaces_to(&mut gauges, 78);
    gauges.push('|');
    lvgl::push_ansi_line(&mut out, gauges.as_str(), theme.text, theme.bg, 84);

    let mut bars = String::from("  | ");
    lvgl::push_meter_text(&mut bars, state.speed_kph, SPEED_MAX_KPH, 20);
    bars.push_str("       |    |     /----\\       |    | ");
    lvgl::push_meter_text(&mut bars, state.rpm, RPM_MAX, 20);
    bars.push_str("       |");
    lvgl::push_ansi_line(&mut out, bars.as_str(), theme.accent, theme.bg, 84);

    lvgl::push_ansi_line(
        &mut out,
        "  |      LVGL arc meter        |    |    | SMROS |      |    |      LVGL arc meter        |",
        theme.line,
        theme.bg,
        84,
    );
    lvgl::push_ansi_line(
        &mut out,
        "  |        FxFS PPM            |    |     \\____/       |    |        Qt window QML       |",
        theme.muted,
        theme.bg,
        84,
    );
    lvgl::push_ansi_line(
        &mut out,
        "  +----------------------------+    +------------------+    +----------------------------+",
        theme.line,
        theme.bg,
        84,
    );

    let mut bottom = String::from("  Range ");
    lvgl::append_usize(&mut bottom, state.range_km, 0);
    bottom.push_str(" km  ");
    lvgl::push_meter_text(&mut bottom, state.range_km, 420, 18);
    bottom.push_str("     Battery ");
    lvgl::append_usize(&mut bottom, state.battery_percent, 0);
    bottom.push_str("% ");
    lvgl::push_meter_text(&mut bottom, state.battery_percent, 100, 18);
    lvgl::push_ansi_line(&mut out, bottom.as_str(), theme.ok, theme.bg, 84);

    let mut warn = String::from("  Warning ");
    warn.push_str(state.warning.as_str());
    warn.push_str("  Source ");
    warn.push_str(QML_CLUSTER_QML_PATH);
    lvgl::push_ansi_line(&mut out, warn.as_str(), theme.text, theme.bg, 84);
    out.push_str("\x1b[0m");
    out
}

fn push_spaces_to(out: &mut String, target_len: usize) {
    while out.len() < target_len {
        out.push(' ');
    }
}

fn sx(value: usize) -> usize {
    value.saturating_mul(CLUSTER_RENDER_WIDTH) / CLUSTER_DESIGN_WIDTH
}

fn sy(value: usize) -> usize {
    value.saturating_mul(CLUSTER_RENDER_HEIGHT) / CLUSTER_DESIGN_HEIGHT
}

fn sr(value: usize) -> usize {
    let x = sx(value);
    let y = sy(value);
    x.min(y).max(1)
}

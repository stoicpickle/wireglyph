use std::{env, error::Error, fmt::Write as _, fs, path::PathBuf};

use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Cell,
    style::{Color, Modifier},
};
use wireglyph::{App, MotionMode, model::Graph, theme::ThemeName, ui};

fn main() -> Result<(), Box<dyn Error>> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("proof/wireglyph-140x40.svg"));
    let width = env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(140);
    let height = env::args()
        .nth(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    let theme = match env::args().nth(4).as_deref() {
        Some("green") => ThemeName::GreenRadar,
        Some("cyan") => ThemeName::CyanAnalyzer,
        Some("red") => ThemeName::RedSchematic,
        Some("mono") => ThemeName::MonoField,
        _ => ThemeName::AmberPlotter,
    };
    let state = env::args().nth(5);
    let flow_hop = state
        .as_deref()
        .filter(|value| *value != "play")
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|hop| hop.checked_sub(1));
    let playback_motion = match env::args().nth(6).as_deref() {
        Some("reduced") => MotionMode::Reduced,
        Some("off") => MotionMode::Off,
        _ => MotionMode::Full,
    };
    let playback_ticks = env::args()
        .nth(7)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    let mut app = if let Some(graph_path) = env::var_os("WIREGLYPH_GRAPH") {
        let graph: Graph = serde_json::from_str(&fs::read_to_string(graph_path)?)?;
        App::from_graph(graph)
    } else {
        App::new()?
    };
    app.set_theme(theme);
    if let Some(hop) = flow_hop {
        let _ = app.select_static_flow_hop(hop);
    }
    if state.as_deref() == Some("trace") {
        let _ = app.arm_selected_trace();
    }
    if state.as_deref() == Some("play") {
        app.set_motion_mode(playback_motion);
        if playback_motion == MotionMode::Off {
            let _ = app.select_static_flow_hop(playback_ticks);
        } else {
            let _ = app.start_static_playback();
            for _ in 0..playback_ticks {
                let _ = app.advance_static_playback();
            }
        }
    }
    terminal.draw(|frame| ui::render(frame, &app))?;

    let svg = render_svg(terminal.backend().buffer().content(), width, height);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, svg)?;
    Ok(())
}

fn render_svg(cells: &[Cell], width: u16, height: u16) -> String {
    const CELL_W: u16 = 10;
    const CELL_H: u16 = 19;
    let pixel_width = u32::from(width) * u32::from(CELL_W);
    let pixel_height = u32::from(height) * u32::from(CELL_H);
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{pixel_width}" height="{pixel_height}" viewBox="0 0 {pixel_width} {pixel_height}"><rect width="100%" height="100%" fill="#050505"/><g font-family="Menlo, Monaco, monospace" font-size="15" dominant-baseline="hanging">"##
    );

    for (index, cell) in cells.iter().enumerate() {
        let x = (index % usize::from(width)) as u16 * CELL_W;
        let y = (index / usize::from(width)) as u16 * CELL_H;
        let bg = color_hex(cell.bg, "#050505");
        let fg = color_hex(cell.fg, "#d0d0d0");
        let weight = if cell.modifier.contains(Modifier::BOLD) {
            "700"
        } else {
            "400"
        };
        if bg != "#050505" {
            let _ = write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="{CELL_W}" height="{CELL_H}" fill="{bg}"/>"#
            );
        }
        let symbol = escape_xml(cell.symbol());
        if symbol.trim().is_empty() {
            continue;
        }
        let _ = write!(
            svg,
            r#"<text x="{x}" y="{}" fill="{fg}" font-weight="{weight}">{symbol}</text>"#,
            y + 1
        );
    }
    svg.push_str("</g></svg>");
    svg
}

fn color_hex(color: Color, fallback: &'static str) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#000000".into(),
        Color::White => "#ffffff".into(),
        Color::Gray => "#808080".into(),
        Color::DarkGray => "#404040".into(),
        Color::Red | Color::LightRed => "#ff604c".into(),
        Color::Green | Color::LightGreen => "#79d679".into(),
        Color::Yellow | Color::LightYellow => "#ffd37a".into(),
        Color::Blue | Color::LightBlue => "#58a6ff".into(),
        Color::Magenta | Color::LightMagenta => "#d56bdf".into(),
        Color::Cyan | Color::LightCyan => "#55d6e2".into(),
        _ => fallback.into(),
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

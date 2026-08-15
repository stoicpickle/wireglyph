use ratatui::{Terminal, backend::TestBackend};
use wireglyph::{App, theme::ThemeName, ui};

#[test]
fn instrument_frame_renders_at_spike_target_sizes() {
    for (width, height) in [(100, 30), (140, 40)] {
        let screen = render_screen(width, height, ThemeName::AmberPlotter);

        assert!(screen.contains("WIREGLYPH // BEACON OPS"), "{screen}");
        assert!(
            screen.contains("SYSTEM MAP // CLUSTERED LAYER 01"),
            "{screen}"
        );
        assert!(
            screen.contains("STATIC PATH // POSSIBLE STRUCTURAL ROUTE"),
            "{screen}"
        );
        assert!(screen.contains("◆ SERVER"), "{screen}");
        assert!(screen.contains("AMBER // PLOTTER"), "{screen}");
        assert!(screen.contains("Q  EXIT"), "{screen}");
        assert!(screen.contains("F FLOW"), "{screen}");
        assert!(
            screen.contains("E EVIDENCE") || screen.contains("E DRAWER"),
            "{screen}"
        );
    }
}

#[test]
fn undersized_terminal_gets_an_explicit_bail_screen() {
    let screen = render_screen(80, 24, ThemeName::AmberPlotter);

    assert!(screen.contains("WIREGLYPH // DISPLAY LIMIT"));
    assert!(screen.contains("CURRENT  080 × 24"));
    assert!(screen.contains("REQUIRED 100 × 30"));
}

#[test]
fn fixed_size_layouts_match_the_reviewed_goldens() {
    insta::assert_snapshot!(
        "amber_100x30",
        render_screen(100, 30, ThemeName::AmberPlotter)
    );
    insta::assert_snapshot!(
        "amber_140x40",
        render_screen(140, 40, ThemeName::AmberPlotter)
    );
}

#[test]
fn every_theme_renders_all_semantic_color_roles() {
    for theme in ThemeName::ALL {
        let width = 100;
        let height = 30;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let mut app = App::new().expect("fixture should load");
        app.set_theme(theme);
        terminal
            .draw(|frame| ui::render(frame, &app))
            .expect("frame should render");

        let palette = theme.palette();
        let cells = terminal.backend().buffer().content();
        for color in [
            palette.primary,
            palette.hot,
            palette.secondary,
            palette.text,
            palette.muted,
            palette.grid,
            palette.warning,
            palette.inferred,
        ] {
            assert!(
                cells
                    .iter()
                    .any(|cell| cell.fg == color || cell.bg == color),
                "{} did not render role {color:?}",
                theme.label()
            );
        }
    }
}

fn render_screen(width: u16, height: u16, theme: ThemeName) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
    let mut app = App::new().expect("fixture should load");
    app.set_theme(theme);

    terminal
        .draw(|frame| ui::render(frame, &app))
        .expect("frame should render");

    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

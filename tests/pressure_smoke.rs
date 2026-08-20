use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use ratatui::{Terminal, backend::TestBackend};
use wireglyph::{App, scanner::scan_project, ui};

struct TempProject(PathBuf);

impl TempProject {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wireglyph-pressure-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("pressure fixture should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, relative: &str, contents: impl AsRef<[u8]>) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent should be created");
        }
        fs::write(path, contents).expect("fixture source should be written");
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scanner_accepts_the_exact_file_limit_and_refuses_the_next_file() {
    let project = TempProject::new("files");
    for index in 0..40 {
        let source = if index < 39 {
            format!("import './file{:02}';\n", index + 1)
        } else {
            "export const end = true;\n".into()
        };
        project.write(&format!("file{index:02}.ts"), source);
    }

    let graph = scan_project(project.path()).expect("40 source files should remain supported");
    assert_eq!(graph.scan_summary.files_scanned, 40);
    assert_eq!(graph.nodes.len(), 40);
    assert_eq!(graph.edges.len(), 39);

    project.write("file40.ts", "export const over = true;\n");
    let error = scan_project(project.path()).expect_err("41 source files must be refused");
    assert!(
        error
            .to_string()
            .contains("41 supported source files exceed the readable first-version limit of 40"),
        "{error}"
    );
}

#[test]
fn dense_graph_accepts_400_edges_traces_and_refuses_edge_401() {
    let project = TempProject::new("edges");
    for source in 0..20 {
        let mut contents = String::new();
        for target in 0..20 {
            if source != target {
                contents.push_str(&format!("import './file{target:02}';\n"));
            }
        }
        contents.push_str(&format!("import './file{:02}';\n", (source + 1) % 20));
        project.write(&format!("file{source:02}.ts"), contents);
    }

    let mut graph = scan_project(project.path()).expect("400 edges should remain supported");
    assert_eq!(graph.nodes.len(), 20);
    assert_eq!(graph.edges.len(), 400);
    graph.repository = "資料視覚化リポジトリ".repeat(20);

    let mut app = App::from_graph(graph);
    assert!(app.arm_selected_trace());
    assert!(app.start_static_playback());
    for _ in 0..500 {
        let _ = app.advance_static_playback();
    }

    for (width, height) in [(1, 1), (99, 29), (100, 30), (127, 35), (128, 36), (200, 60)] {
        let screen = render(&app, width, height);
        if width >= 100 && height >= 30 {
            let header = screen.lines().take(3).collect::<Vec<_>>().join("\n");
            assert!(
                header.contains("STATIC PATH"),
                "header lost path status at {width}x{height}: {header}"
            );
            assert!(
                header.contains("AMBER // PLOTTER"),
                "header lost theme status at {width}x{height}: {header}"
            );
            assert!(
                screen.contains("NOT RUNTIME DATA") || screen.contains("NOT OBSERVED RUNTIME DATA"),
                "{width}x{height}: {screen}"
            );
            assert!(screen.contains("Q  EXIT"), "{width}x{height}: {screen}");
        } else if width == 99 && height == 29 {
            assert!(screen.contains("DISPLAY LIMIT"), "{screen}");
        }
    }

    let mut file_zero = String::new();
    for target in 1..20 {
        file_zero.push_str(&format!("import './file{target:02}';\n"));
    }
    file_zero.push_str("import './file01';\nimport './file02';\n");
    project.write("file00.ts", file_zero);
    let error = scan_project(project.path()).expect_err("401 edges must be refused");
    assert!(
        error
            .to_string()
            .contains("401 static import edges exceed the first-version limit of 400"),
        "{error}"
    );
}

#[test]
fn oversized_binary_and_mixed_supported_sources_are_reported_without_crashing() {
    let project = TempProject::new("skips");
    project.write("oversized.ts", vec![b' '; 512 * 1024 + 1]);
    project.write("binary.py", b"value = 1\0hidden = 2\n");
    project.write("notes.rs", "fn main() {}\n");
    project.write("main.ts", "export const main = true;\n");

    let graph = scan_project(project.path()).expect("unsafe inputs should be skipped explicitly");
    assert_eq!(graph.scan_summary.files_discovered, 4);
    assert_eq!(graph.scan_summary.files_scanned, 2);
    assert_eq!(graph.scan_summary.files_skipped, 2);
    assert_eq!(graph.scan_summary.skipped_by_reason["oversized"], 1);
    assert_eq!(graph.scan_summary.skipped_by_reason["binary"], 1);
    assert_eq!(graph.nodes.len(), 2);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.evidence.path == "notes.rs")
    );
}

#[test]
fn json_cli_defaults_to_working_directory_and_rejects_a_missing_root() {
    let project = TempProject::new("cli");
    project.write("main.ts", "import './worker';\n");
    project.write("worker.ts", "export const worker = true;\n");

    let output = Command::new(env!("CARGO_BIN_EXE_wireglyph"))
        .arg("scan")
        .current_dir(project.path())
        .output()
        .expect("JSON scan command should run");
    assert!(output.status.success(), "{:?}", output.status);
    let graph: wireglyph::model::Graph =
        serde_json::from_slice(&output.stdout).expect("stdout should contain only graph JSON");
    assert_eq!(graph.scan_summary.files_scanned, 2);
    assert_eq!(graph.edges.len(), 1);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("scan SCOPED OK: 2/2 files, 2 nodes, 1 static edges")
    );

    let missing = project.path().join("missing");
    let output = Command::new(env!("CARGO_BIN_EXE_wireglyph"))
        .args(["scan", missing.to_str().expect("temp path should be UTF-8")])
        .output()
        .expect("invalid-root command should run");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid project"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let unsupported = TempProject::new("unsupported-cli");
    unsupported.write("main.go", "package main\n\nfunc main() {}\n");
    let output = Command::new(env!("CARGO_BIN_EXE_wireglyph"))
        .args([
            "scan",
            unsupported
                .path()
                .to_str()
                .expect("temp path should be UTF-8"),
        ])
        .output()
        .expect("unsupported-source command should run");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "failed scans must not emit JSON");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "no supported Rust, JavaScript/TypeScript, or Python source files were found; choose a supported package or subdirectory"
        ),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn render(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
    terminal
        .draw(|frame| ui::render(frame, app))
        .expect("pressure frame should render");
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

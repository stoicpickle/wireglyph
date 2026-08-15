use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use wireglyph::{
    model::Provenance,
    path_export::{PATH_EXPORT_ARTIFACT_TYPE, PATH_EXPORT_SCHEMA_VERSION, PathExport},
};

struct TempProject(PathBuf);

impl TempProject {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wireglyph-path-export-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("path-export fixture should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, relative: &str, contents: &str) {
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
fn cli_emits_a_deterministic_portable_static_path_artifact() {
    let project = TempProject::new("determinism");
    project.write("src/main.ts", "import './worker';\n");
    project.write("src/worker.ts", "import './leaf';\n");
    project.write("src/leaf.ts", "export const leaf = true;\n");

    let first = run_path(&project, "./src/main.ts");
    let second = run_path(&project, "src/main.ts");
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(second.status.success(), "{}", stderr(&second));
    assert_eq!(first.stdout, second.stdout);
    assert!(!first.stdout.is_empty());

    let artifact: PathExport =
        serde_json::from_slice(&first.stdout).expect("stdout should contain only path JSON");
    assert_eq!(artifact.artifact_type, PATH_EXPORT_ARTIFACT_TYPE);
    assert_eq!(artifact.schema_version, PATH_EXPORT_SCHEMA_VERSION);
    assert_eq!(artifact.source_graph_schema_version, 2);
    assert_eq!(artifact.selector, "src/main.ts");
    assert_eq!(artifact.flow.edge_ids.len(), 2);
    assert_eq!(artifact.nodes.len(), 3);
    assert_eq!(artifact.edges.len(), 2);
    assert!(
        artifact
            .edges
            .iter()
            .all(|edge| edge.provenance == Provenance::Extracted)
    );
    assert!(
        artifact
            .nodes
            .iter()
            .all(|node| !Path::new(&node.evidence.path).is_absolute())
    );
    assert!(
        artifact
            .edges
            .iter()
            .all(|edge| !Path::new(&edge.evidence.path).is_absolute())
    );

    let checkout_root = project.path().to_string_lossy();
    assert!(!String::from_utf8_lossy(&first.stdout).contains(checkout_root.as_ref()));
    assert!(
        stderr(&first).contains("static path SCOPED OK: 2 hops from src/main.ts"),
        "{}",
        stderr(&first)
    );
}

#[test]
fn cli_path_errors_never_emit_partial_json() {
    let project = TempProject::new("errors");
    project.write("main.ts", "import './leaf';\n");
    project.write("leaf.ts", "export const leaf = true;\n");
    project.write("absolute.ts", "import '/Users/example/private/token';\n");
    project.write(
        "windows.ts",
        r"import 'C:\Users\example\private\token';
",
    );

    for selector in ["missing.ts", "leaf.ts", "../main.ts"] {
        let output = run_path(&project, selector);
        assert!(
            !output.status.success(),
            "selector unexpectedly worked: {selector}"
        );
        assert!(output.stdout.is_empty(), "partial JSON for {selector}");
        assert!(
            stderr(&output).starts_with("wireglyph: "),
            "{}",
            stderr(&output)
        );
    }

    for (selector, private_literal) in [
        ("absolute.ts", "/Users/example/private/token"),
        ("windows.ts", r"C:\Users\example\private\token"),
    ] {
        let absolute = run_path(&project, selector);
        assert!(!absolute.status.success());
        assert!(absolute.stdout.is_empty());
        assert!(!stderr(&absolute).contains(private_literal));
        assert!(
            stderr(&absolute).contains("refusing portable export"),
            "{selector}: {}",
            stderr(&absolute)
        );
    }
}

#[test]
fn cli_treats_a_closed_path_export_pipe_as_a_clean_exit() {
    let project = TempProject::new("broken-pipe");
    project.write("main.ts", "import './leaf';\n");
    project.write("leaf.ts", "export const leaf = true;\n");

    let mut child = Command::new(env!("CARGO_BIN_EXE_wireglyph"))
        .args([
            "path",
            project.path().to_str().expect("temp path should be UTF-8"),
            "--from",
            "main.ts",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("path command should start");
    drop(child.stdout.take());
    let output = child
        .wait_with_output()
        .expect("path command should observe the closed pipe");

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
}

fn run_path(project: &TempProject, selector: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wireglyph"))
        .args([
            "path",
            project.path().to_str().expect("temp path should be UTF-8"),
            "--from",
            selector,
            "--format",
            "json",
        ])
        .output()
        .expect("path command should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

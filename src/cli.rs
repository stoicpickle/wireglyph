use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "wireglyph",
    version,
    about = "A terminal-native map of a project's static module relationships",
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Project directory to scan before opening the TUI.
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Emit deterministic graph JSON to stdout without opening the TUI.
    Scan {
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },
    /// Emit one deterministic evidence-backed static path without opening the TUI.
    Path {
        #[arg(value_name = "PATH", default_value = ".")]
        project: PathBuf,
        /// Repository-relative source file from which to build the static path.
        #[arg(long, value_name = "SOURCE_PATH")]
        from: String,
        #[arg(long, value_enum, default_value = "json")]
        format: OutputFormat,
    },
    /// Open the built-in BEACON OPS visual demonstration.
    Demo,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Invocation {
    Project(PathBuf),
    Json(PathBuf),
    Path {
        project: PathBuf,
        from: String,
        format: OutputFormat,
    },
    Demo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Json,
}

impl Cli {
    pub fn invocation(self) -> Invocation {
        match (self.command, self.path) {
            (Some(Command::Scan { path }), _) => Invocation::Json(path),
            (
                Some(Command::Path {
                    project,
                    from,
                    format,
                }),
                _,
            ) => Invocation::Path {
                project,
                from,
                format,
            },
            (Some(Command::Demo), _) => Invocation::Demo,
            (None, path) => Invocation::Project(path.unwrap_or_else(|| ".".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Invocation, OutputFormat};

    #[test]
    fn accepts_direct_project_scan_and_json_scan_forms() {
        let current_directory = Cli::try_parse_from(["wireglyph"]).unwrap();
        assert_eq!(
            current_directory.invocation(),
            Invocation::Project(".".into())
        );

        let direct = Cli::try_parse_from(["wireglyph", "../project"]).unwrap();
        assert_eq!(
            direct.invocation(),
            Invocation::Project("../project".into())
        );

        let json = Cli::try_parse_from(["wireglyph", "scan", "../project"]).unwrap();
        assert_eq!(json.invocation(), Invocation::Json("../project".into()));

        let demo = Cli::try_parse_from(["wireglyph", "demo"]).unwrap();
        assert_eq!(demo.invocation(), Invocation::Demo);

        let path = Cli::try_parse_from([
            "wireglyph",
            "path",
            "../project",
            "--from",
            "src/main.rs",
            "--format",
            "json",
        ])
        .unwrap();
        assert_eq!(
            path.invocation(),
            Invocation::Path {
                project: "../project".into(),
                from: "src/main.rs".into(),
                format: OutputFormat::Json,
            }
        );
    }

    #[test]
    fn json_scan_defaults_to_the_current_directory() {
        let cli = Cli::try_parse_from(["wireglyph", "scan"]).unwrap();

        assert_eq!(cli.invocation(), Invocation::Json(".".into()));
    }

    #[test]
    fn path_export_defaults_to_current_directory_and_json() {
        let cli = Cli::try_parse_from(["wireglyph", "path", "--from", "src/main.rs"]).unwrap();

        assert_eq!(
            cli.invocation(),
            Invocation::Path {
                project: ".".into(),
                from: "src/main.rs".into(),
                format: OutputFormat::Json,
            }
        );
    }

    #[test]
    fn path_export_requires_a_source_and_rejects_unknown_formats() {
        let missing = Cli::try_parse_from(["wireglyph", "path"]).unwrap_err();
        assert_eq!(
            missing.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );

        let unknown = Cli::try_parse_from([
            "wireglyph",
            "path",
            "--from",
            "src/main.rs",
            "--format",
            "yaml",
        ])
        .unwrap_err();
        assert_eq!(unknown.kind(), clap::error::ErrorKind::InvalidValue);
    }
}

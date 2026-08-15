use std::{
    error::Error,
    io::{self, Write},
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use clap::Parser;
use serde::Serialize;
use wireglyph::{
    App,
    cli::{Cli, Invocation, OutputFormat},
    path_export::export_selected_static_path,
    scanner::scan_project,
};

#[cfg(feature = "terminal-test-hooks")]
const TERMINAL_EXIT_PROBE_ENV: &str = "WIREGLYPH_TEST_TERMINAL_EXIT";

#[cfg(feature = "terminal-test-hooks")]
#[derive(Clone, Copy)]
enum TerminalExitProbe {
    Ok,
    Error,
    Panic,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("wireglyph: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let app = match Cli::parse().invocation() {
        Invocation::Json(path) => {
            let graph = scan_project(path)?;
            if !write_pretty_json(&graph)? {
                return Ok(());
            }
            eprintln!(
                "wireglyph: scan {}: {}/{} files, {} nodes, {} static edges, {} skipped, {} parse warnings, {} traversal errors",
                graph.scan_summary.health_label(),
                graph.scan_summary.files_scanned,
                graph.scan_summary.files_discovered,
                graph.nodes.len(),
                graph.edges.len(),
                graph.scan_summary.files_skipped,
                graph.scan_summary.parse_warnings,
                graph.scan_summary.traversal_errors,
            );
            return Ok(());
        }
        Invocation::Path {
            project,
            from,
            format,
        } => {
            let graph = scan_project(project)?;
            let export = export_selected_static_path(&graph, &from)?;
            match format {
                OutputFormat::Json if !write_pretty_json(&export)? => return Ok(()),
                OutputFormat::Json => {}
            }
            eprintln!(
                "wireglyph: static path {}: {} hops from {}, {} edge explorations{}{}",
                graph.scan_summary.health_label(),
                export.flow.edge_ids.len(),
                export.selector,
                export.path_search.edge_explorations,
                if export.path_search.hop_limit_reached {
                    "; hop limit reached"
                } else {
                    ""
                },
                if export.path_search.edge_exploration_limit_reached {
                    "; exploration limit reached"
                } else {
                    ""
                },
            );
            return Ok(());
        }
        Invocation::Project(path) => App::from_graph(scan_project(path)?),
        Invocation::Demo => App::new()?,
    };
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&interrupted);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::Relaxed))?;
    let app = app.with_interrupt_flag(interrupted);
    #[cfg(feature = "terminal-test-hooks")]
    {
        let exit_probe = terminal_exit_probe()?;
        ratatui::run(|terminal| match exit_probe {
            Some(TerminalExitProbe::Ok) => {
                terminal.draw(|frame| wireglyph::ui::render(frame, &app))?;
                Ok(())
            }
            Some(TerminalExitProbe::Error) => {
                terminal.draw(|frame| wireglyph::ui::render(frame, &app))?;
                Err(io::Error::other("injected terminal failure"))
            }
            Some(TerminalExitProbe::Panic) => {
                terminal.draw(|frame| wireglyph::ui::render(frame, &app))?;
                panic!("injected terminal panic");
            }
            None => app.run(terminal),
        })?;
    }
    #[cfg(not(feature = "terminal-test-hooks"))]
    ratatui::run(|terminal| app.run(terminal))?;
    Ok(())
}

fn write_pretty_json(value: &impl Serialize) -> Result<bool, Box<dyn Error>> {
    let rendered = serde_json::to_string_pretty(value)?;
    match writeln!(io::stdout().lock(), "{rendered}") {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "terminal-test-hooks")]
fn terminal_exit_probe() -> Result<Option<TerminalExitProbe>, Box<dyn Error>> {
    let Some(value) = std::env::var_os(TERMINAL_EXIT_PROBE_ENV) else {
        return Ok(None);
    };
    match value.to_str() {
        Some("ok") => Ok(Some(TerminalExitProbe::Ok)),
        Some("error") => Ok(Some(TerminalExitProbe::Error)),
        Some("panic") => Ok(Some(TerminalExitProbe::Panic)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{TERMINAL_EXIT_PROBE_ENV} accepts only `ok`, `error`, or `panic`"),
        )
        .into()),
    }
}

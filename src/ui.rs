use std::collections::{BTreeMap, HashSet};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line as TextLine, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap, canvas},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    App,
    graph_layout::{Point, canvas_to_cell},
    map_view::{GroupLinkSummary, MapMode, MapRenderPlan, build_render_plan, root_candidate_ids},
    model::{Node, NodeKind, Provenance},
    theme::Palette,
};

const MIN_WIDTH: u16 = 100;
const MIN_HEIGHT: u16 = 30;
const WIDE_WIDTH: u16 = 128;
const WIDE_HEIGHT: u16 = 36;

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let palette = app.theme.palette();
    frame.render_widget(
        Block::new().style(Style::new().bg(palette.background)),
        frame.area(),
    );

    if frame.area().width < MIN_WIDTH || frame.area().height < MIN_HEIGHT {
        render_minimum_size(frame, palette);
        return;
    }

    let [header, body, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .areas(frame.area());

    render_header(frame, header, app, palette);
    let wide = frame.area().width >= WIDE_WIDTH && frame.area().height >= WIDE_HEIGHT;
    if wide {
        render_wide_body(frame, body, app, palette);
    } else {
        render_map(frame, body, app, palette);
        render_compact_drawer(frame, body, app, palette);
    }
    render_footer(frame, footer, app, palette, wide);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    let hop = selected_flow_edge(app).map_or(0, |(hop, _, _, _, _)| hop + 1);
    let total_hops = app.current_flow().map_or(0, |flow| flow.edge_ids.len());
    let playback = match (app.playback, app.motion, app.flow_hop) {
        (_, _, None) => "ARM",
        (_, crate::MotionMode::Off, Some(_)) => "MOTION OFF",
        (crate::PlaybackState::Paused, _, Some(_)) => "PAUSE",
        (crate::PlaybackState::Playing, crate::MotionMode::Full, Some(_)) => "PLAY",
        (crate::PlaybackState::Playing, crate::MotionMode::Reduced, Some(_)) => "REDUCED",
    };
    let status = if app.current_flow().is_none() && is_fixture(app) {
        format!(
            "{:02}N {:02}E  {}  STATIC IMPORTS",
            app.graph.nodes.len(),
            app.graph.edges.len(),
            app.graph.scan_summary.health_label(),
        )
    } else if is_fixture(app) {
        format!(
            "{:02}N {:02}E  STATIC F01  {playback}  {hop:02}/{total_hops:02}",
            app.graph.nodes.len(),
            app.graph.edges.len()
        )
    } else {
        match app.map_mode {
            MapMode::Overview => format!(
                "{:02}N {:02}E  {}  OVERVIEW",
                app.graph.nodes.len(),
                app.graph.edges.len(),
                app.graph.scan_summary.health_label(),
            ),
            MapMode::Focus => format!(
                "{:02}N {:02}E  {}  FOCUS",
                app.graph.nodes.len(),
                app.graph.edges.len(),
                app.graph.scan_summary.health_label(),
            ),
            MapMode::Trace => format!(
                "{:02}N {:02}E  {}  STATIC PATH  {playback}  {hop:02}/{total_hops:02}",
                app.graph.nodes.len(),
                app.graph.edges.len(),
                app.graph.scan_summary.health_label(),
            ),
        }
    };
    const PREFIX: &str = " WIREGLYPH // ";
    let status_span = format!("   {status}   ");
    let fixed_width = PREFIX.width() + status_span.width() + app.theme.label().width();
    let repository_width = usize::from(area.width.saturating_sub(2)).saturating_sub(fixed_width);
    let repository = truncate(&app.graph.repository, repository_width);
    frame.render_widget(
        Paragraph::new(TextLine::from(vec![
            Span::styled(PREFIX, Style::new().fg(palette.muted)),
            Span::styled(
                repository,
                Style::new().fg(palette.hot).add_modifier(Modifier::BOLD),
            ),
            Span::styled(status_span, Style::new().fg(palette.warning)),
            Span::styled(app.theme.label(), Style::new().fg(palette.primary)),
        ]))
        .block(instrument_block(
            "SYSTEM STATUS",
            palette.primary,
            palette.panel,
        )),
        area,
    );
}

fn render_wide_body(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    let [navigator, map, inspector] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22),
            Constraint::Min(50),
            Constraint::Length(34),
        ])
        .areas(area);
    render_navigator(frame, navigator, app, palette);
    render_map(frame, map, app, palette);
    render_inspector(frame, inspector, app, palette);
}

fn render_compact_drawer(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    match app.active_panel {
        crate::Panel::Map => {}
        crate::Panel::Navigator => {
            let width = 28.min(area.width);
            let drawer = Rect::new(area.x, area.y, width, area.height);
            frame.render_widget(Clear, drawer);
            render_navigator(frame, drawer, app, palette);
        }
        crate::Panel::Inspector => {
            let width = 44.min(area.width);
            let drawer = Rect::new(
                area.right().saturating_sub(width),
                area.y,
                width,
                area.height,
            );
            frame.render_widget(Clear, drawer);
            render_inspector(frame, drawer, app, palette);
        }
    }
}

fn render_navigator(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    if !is_fixture(app) {
        render_project_navigator(frame, area, app, palette);
        return;
    }
    let selected_group = selected_node(app).map(|node| node.group.as_str());
    let group_line = |group: &'static str, count: usize| {
        let marker = if selected_group == Some(group) {
            "▶"
        } else {
            " "
        };
        let style = if selected_group == Some(group) {
            Style::new().fg(palette.hot).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(palette.text)
        };
        TextLine::styled(format!("{marker} {group:<8} {count:02}"), style)
    };
    let lines = vec![
        TextLine::styled("◆ ENTRY POINTS  02", Style::new().fg(palette.hot)),
        TextLine::styled("  SERVER.ENTRY", Style::new().fg(palette.text)),
        TextLine::styled("  WEB.ENTRY", Style::new().fg(palette.muted)),
        TextLine::from(""),
        TextLine::styled("□ SUBSYSTEMS    07", Style::new().fg(palette.primary)),
        group_line("BOOT", 3),
        group_line("HTTP", 5),
        group_line("DOMAIN", 5),
        group_line("DATA", 5),
        group_line("UI", 5),
        group_line("EXT", 4),
        group_line("OPS", 2),
        TextLine::from(""),
        TextLine::styled("▷ STATIC FLOW  01", Style::new().fg(palette.warning)),
        TextLine::styled("  SYSTEM DETAIL", Style::new().fg(palette.hot)),
        TextLine::styled("  09 HOPS", Style::new().fg(palette.muted)),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(instrument_block(
                if app.active_panel == crate::Panel::Navigator {
                    "NAVIGATOR // ACTIVE"
                } else {
                    "NAVIGATOR"
                },
                if app.active_panel == crate::Panel::Navigator {
                    palette.hot
                } else {
                    palette.secondary
                },
                palette.panel,
            ))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_project_navigator(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    let selected_group = selected_node(app).map(|node| node.group.as_str());
    let entries: Vec<_> = app
        .graph
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Entry))
        .collect();
    let root_ids = root_candidate_ids(&app.graph);
    let roots: Vec<_> = root_ids
        .iter()
        .filter_map(|id| app.graph.nodes.iter().find(|node| node.id == *id))
        .collect();
    let (anchor_heading, anchors) = if entries.is_empty() {
        ("ROOT CANDIDATES", &roots)
    } else {
        ("ENTRY POINTS", &entries)
    };
    let mut group_counts = BTreeMap::new();
    for node in &app.graph.nodes {
        *group_counts.entry(node.group.as_str()).or_insert(0_usize) += 1;
    }
    let groups: Vec<_> = group_counts.into_iter().collect();
    let mut lines = vec![TextLine::styled(
        format!("◆ {anchor_heading:<15} {:02}", anchors.len()),
        Style::new().fg(palette.hot),
    )];
    for node in anchors.iter().take(2) {
        lines.push(TextLine::styled(
            format!("  {}", truncate(&node.label, 17)),
            Style::new().fg(palette.text),
        ));
    }
    if anchors.len() > 2 {
        lines.push(TextLine::styled(
            format!("  +{:02} MORE", anchors.len() - 2),
            Style::new().fg(palette.muted),
        ));
    }
    lines.push(TextLine::from(""));
    lines.push(TextLine::styled(
        format!("□ SUBSYSTEMS    {:02}", groups.len()),
        Style::new().fg(palette.primary),
    ));
    let mut suffix = vec![TextLine::from("")];
    let health = app.graph.scan_summary.health();
    suffix.push(TextLine::styled(
        format!("◇ SCAN {}", app.graph.scan_summary.health_label()),
        Style::new()
            .fg(if matches!(health, crate::model::ScanHealth::Partial) {
                palette.warning
            } else {
                palette.primary
            })
            .add_modifier(Modifier::BOLD),
    ));
    suffix.push(TextLine::styled(
        format!(
            "  {:02}/{:02} FILES",
            app.graph.scan_summary.files_scanned, app.graph.scan_summary.files_discovered
        ),
        Style::new().fg(palette.text),
    ));
    suffix.push(TextLine::styled(
        format!(
            "  S{:02} P{:02} T{:02}",
            app.graph.scan_summary.files_skipped,
            app.graph.scan_summary.parse_warnings,
            app.graph.scan_summary.traversal_errors,
        ),
        Style::new().fg(palette.muted),
    ));
    suffix.push(TextLine::from(""));
    if let Some(flow) = app.current_flow() {
        let origin = flow
            .node_ids
            .first()
            .and_then(|id| app.graph.nodes.iter().find(|node| node.id == *id))
            .map_or("?", |node| node.label.as_str());
        suffix.push(TextLine::styled(
            format!("▷ OUTWARD PATH  {:02}", flow.edge_ids.len()),
            Style::new().fg(palette.warning),
        ));
        suffix.push(TextLine::styled(
            format!("  FROM {}", truncate(&origin.to_uppercase(), 14)),
            Style::new().fg(palette.hot),
        ));
        suffix.push(TextLine::styled(
            "  STATIC IMPORTS",
            Style::new().fg(palette.text),
        ));
    } else {
        suffix.push(TextLine::styled(
            match app.map_mode {
                MapMode::Overview => "▷ OVERVIEW",
                MapMode::Focus => "▷ FOCUS",
                MapMode::Trace => "▷ PATH READY",
            },
            Style::new().fg(palette.warning),
        ));
        suffix.push(TextLine::styled(
            if app.map_mode == MapMode::Overview {
                "  GROUP LINKS"
            } else {
                "  F OUTWARD"
            },
            Style::new().fg(palette.text),
        ));
    }
    suffix.push(TextLine::styled(
        "  NOT RUNTIME DATA",
        Style::new().fg(palette.muted),
    ));

    let inner_height = usize::from(area.height.saturating_sub(2));
    let group_rows = inner_height.saturating_sub(lines.len() + suffix.len());
    let selected_index = groups
        .iter()
        .position(|(group, _)| selected_group == Some(*group))
        .unwrap_or(0);
    let item_slots = if groups.len() > group_rows {
        group_rows.saturating_sub(2).max(1)
    } else {
        group_rows
    };
    let mut start = selected_index.saturating_sub(item_slots / 2);
    start = start.min(groups.len().saturating_sub(item_slots));
    let end = (start + item_slots).min(groups.len());
    if start > 0 {
        lines.push(TextLine::styled(
            format!("  +{start:02} ABOVE"),
            Style::new().fg(palette.muted),
        ));
    }
    for (group, count) in groups.iter().skip(start).take(end.saturating_sub(start)) {
        let selected = selected_group == Some(*group);
        lines.push(TextLine::styled(
            format!(
                "{} {:<13} {count:02}",
                if selected { "▶" } else { " " },
                truncate(&group.to_uppercase(), 13)
            ),
            if selected {
                Style::new().fg(palette.hot).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(palette.text)
            },
        ));
    }
    if end < groups.len() {
        lines.push(TextLine::styled(
            format!("  +{:02} BELOW", groups.len() - end),
            Style::new().fg(palette.muted),
        ));
    }
    lines.extend(suffix);
    frame.render_widget(
        Paragraph::new(lines)
            .block(instrument_block(
                if app.active_panel == crate::Panel::Navigator {
                    "NAVIGATOR // ACTIVE"
                } else {
                    "NAVIGATOR"
                },
                if app.active_panel == crate::Panel::Navigator {
                    palette.hot
                } else {
                    palette.secondary
                },
                palette.panel,
            ))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_inspector(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    let Some(node) = selected_node(app) else {
        frame.render_widget(
            Paragraph::new(vec![
                TextLine::styled("NO NODE SELECTED", Style::new().fg(palette.warning)),
                TextLine::from(""),
                TextLine::styled(
                    "SELECT A GRAPH NODE TO INSPECT EVIDENCE",
                    Style::new().fg(palette.muted),
                ),
            ])
            .block(instrument_block(
                "INSPECTOR",
                palette.secondary,
                palette.panel,
            ))
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    };
    if let Some((hop, total, edge, source, target)) = selected_flow_edge(app) {
        if !is_fixture(app) {
            render_project_path_inspector(
                frame, area, app, hop, total, edge, source, target, palette,
            );
            return;
        }
        let lines = vec![
            TextLine::styled(
                format!("{} {:02}/{total:02}", "STATIC EDGE", hop + 1),
                Style::new()
                    .fg(palette.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            TextLine::styled(
                format!("{} → {}", source.label, target.label),
                Style::new().fg(palette.hot).add_modifier(Modifier::BOLD),
            ),
            TextLine::styled(
                format!("{} // {}", edge.id, edge.relationship).to_uppercase(),
                Style::new().fg(palette.primary),
            ),
            TextLine::styled(
                format!("{} → {}", source.group, target.group),
                Style::new().fg(palette.text),
            ),
            TextLine::from(""),
            TextLine::styled("EDGE EVIDENCE", Style::new().fg(palette.muted)),
            TextLine::styled(&edge.evidence.path, Style::new().fg(palette.text)),
            TextLine::styled(
                format!(
                    "LINES {}–{}",
                    edge.evidence.line_start, edge.evidence.line_end
                ),
                Style::new().fg(palette.primary),
            ),
            TextLine::from(""),
            TextLine::styled(
                format!("PROVENANCE  {:?}", edge.provenance).to_uppercase(),
                Style::new().fg(palette.text),
            ),
            TextLine::styled(
                format!("CONFIDENCE  {:.2}", edge.confidence),
                Style::new().fg(palette.text),
            ),
            TextLine::from(""),
            TextLine::styled(
                "POSSIBLE STRUCTURAL ROUTE",
                Style::new().fg(palette.warning),
            ),
            TextLine::styled(
                "NOT OBSERVED RUNTIME DATA",
                Style::new().fg(palette.warning),
            ),
            TextLine::from(""),
            TextLine::styled("[,] PREV   [.] NEXT", Style::new().fg(palette.muted)),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .block(instrument_block(
                    "INSPECTOR // FLOW EVIDENCE",
                    palette.hot,
                    palette.panel,
                ))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    if !is_fixture(app) {
        if let Some((index, total, edge, target)) = app.selected_relationship_edge() {
            render_project_relationship_inspector(
                frame, area, app, node, index, total, edge, target, palette,
            );
            return;
        }
        render_project_node_inspector(frame, area, app, node, palette);
        return;
    }
    let outgoing = app
        .graph
        .edges
        .iter()
        .filter(|edge| edge.source == node.id)
        .count();
    let incoming = app
        .graph
        .edges
        .iter()
        .filter(|edge| edge.target == node.id)
        .count();
    let lines = vec![
        TextLine::styled("SELECTED NODE", Style::new().fg(palette.muted)),
        TextLine::styled(
            format!("◆ {}", node.label),
            Style::new().fg(palette.hot).add_modifier(Modifier::BOLD),
        ),
        TextLine::styled(
            format!("{} // {:?}", node.id, node.kind).to_uppercase(),
            Style::new().fg(palette.primary),
        ),
        TextLine::from(""),
        TextLine::styled("EVIDENCE", Style::new().fg(palette.muted)),
        TextLine::styled(&node.evidence.path, Style::new().fg(palette.text)),
        TextLine::styled(
            format!(
                "LINES {}–{}",
                node.evidence.line_start, node.evidence.line_end
            ),
            Style::new().fg(palette.primary),
        ),
        TextLine::from(""),
        TextLine::styled("RELATIONSHIPS", Style::new().fg(palette.muted)),
        TextLine::styled(
            format!("IN  {incoming:02}    OUT {outgoing:02}"),
            Style::new().fg(palette.text),
        ),
        TextLine::from(""),
        TextLine::styled(
            "STATIC PATH",
            Style::new()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
        TextLine::styled("POSSIBLE STRUCTURAL ROUTE", Style::new().fg(palette.muted)),
        TextLine::styled(
            "NOT OBSERVED RUNTIME DATA",
            Style::new().fg(palette.warning),
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(instrument_block(
                if app.active_panel == crate::Panel::Inspector {
                    "INSPECTOR // ACTIVE"
                } else {
                    "INSPECTOR"
                },
                if app.active_panel == crate::Panel::Inspector {
                    palette.hot
                } else {
                    palette.secondary
                },
                palette.panel,
            ))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_project_node_inspector(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    node: &Node,
    palette: Palette,
) {
    let outgoing = app.outgoing_edge_indices();
    let incoming = app
        .graph
        .edges
        .iter()
        .filter(|edge| edge.target == node.id)
        .count();
    let width = usize::from(area.width.saturating_sub(4));
    let mut details = vec![
        TextLine::styled("SELECTED MODULE", Style::new().fg(palette.muted)),
        TextLine::styled(
            middle_truncate(&format!("{} {}", node_glyph(node), node.label), width),
            Style::new().fg(palette.hot).add_modifier(Modifier::BOLD),
        ),
        TextLine::styled(
            format!("{} // {:?}", node.group, node.kind).to_uppercase(),
            Style::new().fg(palette.primary),
        ),
        TextLine::from(""),
        TextLine::styled("SOURCE EVIDENCE", Style::new().fg(palette.muted)),
        TextLine::styled(
            middle_truncate(&node.evidence.path, width),
            Style::new().fg(palette.text),
        ),
        TextLine::styled(
            format!(
                "LINES {}–{}",
                node.evidence.line_start, node.evidence.line_end
            ),
            Style::new().fg(palette.primary),
        ),
        TextLine::from(""),
        TextLine::styled(
            format!("IN  {incoming:02}    OUT {:02}", outgoing.len()),
            Style::new().fg(palette.text),
        ),
    ];
    for edge_index in outgoing.iter().take(4) {
        let edge = &app.graph.edges[*edge_index];
        let target_node = app
            .graph
            .nodes
            .iter()
            .find(|candidate| candidate.id == edge.target);
        let target = target_node.map_or("?", |candidate| candidate.label.as_str());
        let unresolved =
            target_node.is_some_and(|candidate| matches!(candidate.kind, NodeKind::Unresolved));
        details.push(TextLine::styled(
            format!("→ {} // {}", truncate(target, 17), edge.evidence.line_start),
            Style::new().fg(if unresolved {
                palette.inferred
            } else {
                palette.text
            }),
        ));
    }
    if outgoing.len() > 4 {
        details.push(TextLine::styled(
            format!("+{:02} MORE // ,. EDGE", outgoing.len() - 4),
            Style::new().fg(palette.muted),
        ));
    } else if !outgoing.is_empty() {
        details.push(TextLine::styled(
            ",. INSPECT EDGE",
            Style::new().fg(palette.muted),
        ));
    }
    if outgoing.is_empty() {
        details.push(TextLine::styled(
            "NO OUTWARD IMPORTS",
            Style::new()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ));
        details.push(TextLine::styled(
            "SELECT ANOTHER MODULE",
            Style::new().fg(palette.muted),
        ));
    }
    let truth = vec![
        TextLine::styled(
            "STATIC IMPORT RELATIONSHIPS",
            Style::new()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
        TextLine::styled(
            "NOT OBSERVED RUNTIME DATA",
            Style::new().fg(palette.warning),
        ),
        TextLine::styled(",. EDGE EVIDENCE", Style::new().fg(palette.muted)),
    ];
    render_bounded_project_inspector(
        frame,
        area,
        if app.active_panel == crate::Panel::Inspector {
            "INSPECTOR // ACTIVE"
        } else {
            "INSPECTOR"
        },
        if app.active_panel == crate::Panel::Inspector {
            palette.hot
        } else {
            palette.secondary
        },
        palette,
        details,
        truth,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_project_path_inspector(
    frame: &mut Frame<'_>,
    area: Rect,
    _app: &App,
    hop: usize,
    total: usize,
    edge: &crate::model::Edge,
    source: &Node,
    target: &Node,
    palette: Palette,
) {
    let width = usize::from(area.width.saturating_sub(4));
    let details = vec![
        TextLine::styled(
            format!("STATIC PATH {:02}/{total:02}", hop + 1),
            Style::new()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
        TextLine::styled(
            middle_truncate(&format!("{} → {}", source.label, target.label), width),
            Style::new().fg(palette.hot).add_modifier(Modifier::BOLD),
        ),
        TextLine::styled(
            middle_truncate(
                &format!("{} // {}", edge.id, edge.relationship).to_uppercase(),
                width,
            ),
            Style::new().fg(palette.primary),
        ),
        TextLine::styled(
            middle_truncate(&format!("{} → {}", source.group, target.group), width),
            Style::new().fg(palette.text),
        ),
        TextLine::from(""),
        TextLine::styled("EDGE EVIDENCE", Style::new().fg(palette.muted)),
        TextLine::styled(
            middle_truncate(&edge.evidence.path, width),
            Style::new().fg(palette.text),
        ),
        TextLine::styled(
            format!(
                "LINES {}–{}",
                edge.evidence.line_start, edge.evidence.line_end
            ),
            Style::new().fg(palette.primary),
        ),
        TextLine::from(""),
        TextLine::styled(
            format!("PROVENANCE  {:?}", edge.provenance).to_uppercase(),
            Style::new().fg(palette.text),
        ),
        TextLine::styled(
            format!("CONFIDENCE  {:.2}", edge.confidence),
            Style::new().fg(palette.text),
        ),
    ];
    let truth = vec![
        TextLine::styled("STATIC IMPORT PATH", Style::new().fg(palette.muted)),
        TextLine::styled(
            "POSSIBLE STRUCTURAL ROUTE",
            Style::new().fg(palette.warning),
        ),
        TextLine::styled(
            "NOT OBSERVED RUNTIME DATA",
            Style::new().fg(palette.warning),
        ),
        TextLine::styled("[,] PREV   [.] NEXT", Style::new().fg(palette.muted)),
    ];
    render_bounded_project_inspector(
        frame,
        area,
        "INSPECTOR // PATH EVIDENCE",
        palette.hot,
        palette,
        details,
        truth,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_project_relationship_inspector(
    frame: &mut Frame<'_>,
    area: Rect,
    _app: &App,
    source: &Node,
    index: usize,
    total: usize,
    edge: &crate::model::Edge,
    target: Option<&Node>,
    palette: Palette,
) {
    let width = usize::from(area.width.saturating_sub(4));
    let target_label = target.map_or("?", |node| node.label.as_str());
    let target_group = target.map_or("?", |node| node.group.as_str());
    let details = vec![
        TextLine::styled(
            format!("RELATIONSHIP {:02}/{total:02}", index + 1),
            Style::new()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
        TextLine::styled(
            middle_truncate(&format!("{} → {target_label}", source.label), width),
            Style::new().fg(palette.hot).add_modifier(Modifier::BOLD),
        ),
        TextLine::styled(
            middle_truncate(
                &format!("{} // {}", edge.id, edge.relationship).to_uppercase(),
                width,
            ),
            Style::new().fg(palette.primary),
        ),
        TextLine::styled(
            middle_truncate(&format!("{} → {target_group}", source.group), width),
            Style::new().fg(palette.text),
        ),
        TextLine::styled(
            middle_truncate(
                &format!(
                    "SPECIFIER  {}",
                    edge.import_specifier.as_deref().unwrap_or("UNKNOWN")
                ),
                width,
            ),
            Style::new().fg(palette.text),
        ),
        TextLine::from(""),
        TextLine::styled("EDGE EVIDENCE", Style::new().fg(palette.muted)),
        TextLine::styled(
            middle_truncate(&edge.evidence.path, width),
            Style::new().fg(palette.text),
        ),
        TextLine::styled(
            format!(
                "LINES {}–{}",
                edge.evidence.line_start, edge.evidence.line_end
            ),
            Style::new().fg(palette.primary),
        ),
        TextLine::from(""),
        TextLine::styled(
            format!("PROVENANCE  {:?}", edge.provenance).to_uppercase(),
            Style::new().fg(palette.text),
        ),
        TextLine::styled(
            format!("CONFIDENCE  {:.2}", edge.confidence),
            Style::new().fg(palette.text),
        ),
    ];
    let truth = vec![
        TextLine::styled(
            "STATIC IMPORT RELATIONSHIP",
            Style::new().fg(palette.warning),
        ),
        TextLine::styled(
            "NOT OBSERVED RUNTIME DATA",
            Style::new().fg(palette.warning),
        ),
        TextLine::styled("[,] PREV   [.] NEXT", Style::new().fg(palette.muted)),
    ];
    render_bounded_project_inspector(
        frame,
        area,
        "INSPECTOR // EDGE EVIDENCE",
        palette.hot,
        palette,
        details,
        truth,
    );
}

fn render_bounded_project_inspector(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &'static str,
    border: Color,
    palette: Palette,
    details: Vec<TextLine<'static>>,
    truth: Vec<TextLine<'static>>,
) {
    let block = instrument_block(title, border, palette.panel);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let truth_height = truth.len().min(usize::from(inner.height)) as u16;
    let [details_area, truth_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(truth_height)])
        .areas(inner);
    frame.render_widget(Paragraph::new(details), details_area);
    frame.render_widget(Paragraph::new(truth), truth_area);
}

fn selected_flow_edge(app: &App) -> Option<(usize, usize, &crate::model::Edge, &Node, &Node)> {
    let hop = app.flow_hop?;
    let flow = app.current_flow()?;
    let edge_id = flow.edge_ids.get(hop)?;
    let edge = app.graph.edges.iter().find(|edge| edge.id == *edge_id)?;
    let source = app.graph.nodes.iter().find(|node| node.id == edge.source)?;
    let target = app.graph.nodes.iter().find(|node| node.id == edge.target)?;
    Some((hop, flow.edge_ids.len(), edge, source, target))
}

fn is_fixture(app: &App) -> bool {
    app.graph.scan_summary.source == "synthetic_fixture"
}

fn selected_node(app: &App) -> Option<&Node> {
    app.selected_node
        .and_then(|index| app.graph.nodes.get(index))
}

fn render_map(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    let flow_edges: HashSet<_> = app
        .current_flow()
        .into_iter()
        .flat_map(|flow| &flow.edge_ids)
        .map(String::as_str)
        .collect();
    let map_title = if is_fixture(app) {
        "SYSTEM MAP // CLUSTERED LAYER 01"
    } else {
        match app.map_mode {
            MapMode::Overview => "SYSTEM MAP // OVERVIEW",
            MapMode::Focus => "SYSTEM MAP // FOCUS",
            MapMode::Trace => "SYSTEM MAP // STATIC PATH",
        }
    };
    let map_block = instrument_block(map_title, palette.primary, palette.background);
    let inner = inner_rect(area);
    let graph = &app.graph;
    let active_edge = selected_flow_edge(app).map(|(_, _, edge, _, _)| edge);
    let active_edge_id = active_edge.map(|edge| edge.id.as_str());
    let focused_edge_id = app
        .selected_relationship_edge()
        .map(|(_, _, edge, _)| edge.id.as_str());
    let active_endpoint = |node_id: &str| {
        active_edge.is_some_and(|edge| edge.source == node_id || edge.target == node_id)
    };
    let flow_position = |edge_id: &str| {
        app.current_flow().and_then(|flow| {
            flow.edge_ids
                .iter()
                .position(|candidate| candidate == edge_id)
        })
    };
    let selected_id = selected_node(app).map(|node| node.id.as_str());
    let render_plan = (!is_fixture(app))
        .then(|| build_render_plan(&app.graph, app.map_mode, selected_id, app.current_flow()));
    let canvas = canvas::Canvas::default()
        .block(map_block)
        .x_bounds([0.0, 100.0])
        .y_bounds([0.0, 60.0])
        .marker(Marker::Braille)
        .background_color(palette.background)
        .paint(|ctx| {
            for x in (10..100).step_by(10) {
                ctx.draw(&canvas::Line::new(
                    f64::from(x),
                    0.0,
                    f64::from(x),
                    60.0,
                    palette.grid,
                ));
            }
            for y in (10..60).step_by(10) {
                ctx.draw(&canvas::Line::new(
                    0.0,
                    f64::from(y),
                    100.0,
                    f64::from(y),
                    palette.grid,
                ));
            }
            for group in app.layout.groups() {
                ctx.draw(&canvas::Rectangle {
                    x: group.x,
                    y: group.y,
                    width: group.width,
                    height: group.height,
                    color: palette.secondary,
                });
            }
            ctx.layer();
            if let Some(plan) = render_plan
                .as_ref()
                .filter(|_| app.map_mode == MapMode::Overview)
            {
                for link in &plan.group_links {
                    if group_link_crosses_unrelated_group(app, plan, link) {
                        continue;
                    }
                    let Some((source, target)) = group_link_points(app, plan, link) else {
                        continue;
                    };
                    let color = if link.inferred == link.total {
                        palette.inferred
                    } else {
                        palette.secondary
                    };
                    if link.inferred == link.total {
                        let dots = dashed_points(source, target);
                        ctx.draw(&canvas::Points {
                            coords: &dots,
                            color,
                        });
                    } else {
                        ctx.draw(&canvas::Line::new(
                            source.x, source.y, target.x, target.y, color,
                        ));
                    }
                }
            }
            for edge in &graph.edges {
                if render_plan
                    .as_ref()
                    .is_some_and(|plan| !plan.visible_edge_ids.contains(edge.id.as_str()))
                {
                    continue;
                }
                let (Some(source), Some(target)) = (
                    app.layout.position(&edge.source),
                    app.layout.position(&edge.target),
                ) else {
                    continue;
                };
                let color = if is_fixture(app) {
                    if active_edge_id == Some(edge.id.as_str()) {
                        palette.hot
                    } else if edge.provenance == Provenance::Inferred {
                        palette.inferred
                    } else if app.layout.is_cycle_edge(&edge.id) {
                        palette.warning
                    } else if flow_edges.contains(edge.id.as_str()) {
                        if let Some(active_hop) = app.flow_hop {
                            if flow_position(&edge.id).is_some_and(|hop| hop < active_hop) {
                                palette.primary
                            } else {
                                palette.secondary
                            }
                        } else {
                            palette.primary
                        }
                    } else if app.current_flow().is_none()
                        && selected_id.is_some_and(|id| edge.source == id || edge.target == id)
                    {
                        palette.primary
                    } else {
                        palette.grid
                    }
                } else {
                    match app.map_mode {
                        MapMode::Overview => palette.grid,
                        MapMode::Focus => {
                            if focused_edge_id == Some(edge.id.as_str()) {
                                palette.hot
                            } else if edge.provenance == Provenance::Inferred {
                                palette.inferred
                            } else if app.layout.is_cycle_edge(&edge.id) {
                                palette.warning
                            } else {
                                palette.primary
                            }
                        }
                        MapMode::Trace => {
                            if active_edge_id == Some(edge.id.as_str()) {
                                palette.hot
                            } else if let Some(active_hop) = app.flow_hop {
                                if flow_position(&edge.id).is_some_and(|hop| hop < active_hop) {
                                    palette.primary
                                } else {
                                    palette.secondary
                                }
                            } else {
                                palette.primary
                            }
                        }
                    }
                };
                if edge.provenance == Provenance::Inferred {
                    let dots = dashed_points(source, target);
                    ctx.draw(&canvas::Points {
                        coords: &dots,
                        color,
                    });
                } else {
                    ctx.draw(&canvas::Line::new(
                        source.x, source.y, target.x, target.y, color,
                    ));
                }
            }
            ctx.layer();
            for node in &graph.nodes {
                if render_plan.as_ref().is_some_and(|plan| {
                    app.map_mode == MapMode::Trace
                        && !plan.emphasized_node_ids.contains(node.id.as_str())
                }) {
                    continue;
                }
                if let Some(point) = app.layout.position(&node.id) {
                    let color = if is_fixture(app) {
                        if Some(node.id.as_str()) == selected_id || active_endpoint(&node.id) {
                            palette.hot
                        } else {
                            palette.primary
                        }
                    } else {
                        match app.map_mode {
                            MapMode::Overview => {
                                if render_plan.as_ref().is_some_and(|plan| {
                                    plan.emphasized_node_ids.contains(node.id.as_str())
                                }) {
                                    palette.hot
                                } else {
                                    palette.grid
                                }
                            }
                            MapMode::Focus => {
                                if Some(node.id.as_str()) == selected_id {
                                    palette.hot
                                } else if render_plan.as_ref().is_some_and(|plan| {
                                    plan.emphasized_node_ids.contains(node.id.as_str())
                                }) {
                                    palette.primary
                                } else {
                                    palette.grid
                                }
                            }
                            MapMode::Trace => {
                                if active_endpoint(&node.id) {
                                    palette.hot
                                } else {
                                    palette.primary
                                }
                            }
                        }
                    };
                    ctx.draw(&canvas::Points {
                        coords: &[(point.x, point.y)],
                        color,
                    });
                }
            }
        });
    frame.render_widget(canvas, area);
    render_group_labels(frame, inner, app, palette, render_plan.as_ref());
    if !is_fixture(app)
        && let Some(plan) = &render_plan
    {
        render_project_direction_markers(frame, inner, app, palette, plan);
        if app.map_mode == MapMode::Overview {
            render_group_link_labels(frame, inner, app, palette, plan);
        }
    }
    render_node_labels(frame, inner, app, palette, render_plan.as_ref());
    if is_fixture(app) {
        render_flow_direction_markers(frame, inner, app, palette);
    }
    render_playback_marker(frame, inner, app, palette);
    if app.current_flow().is_none() {
        if is_fixture(app) {
            render_overlay(
                frame,
                centered_rect(26.min(inner.width), 1, inner),
                "NO STATIC FLOW AVAILABLE",
                Style::new().fg(palette.warning).bg(palette.background),
            );
        } else {
            render_project_annotations(frame, inner, app, palette);
        }
    } else if is_fixture(app) {
        render_map_annotations(frame, inner, palette);
    } else {
        render_project_annotations(frame, inner, app, palette);
    }
}

fn render_playback_marker(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    if app.motion != crate::MotionMode::Full {
        return;
    }
    let Some((_, _, edge, _, _)) = selected_flow_edge(app) else {
        return;
    };
    let (Some(source), Some(target)) = (
        app.layout.position(&edge.source),
        app.layout.position(&edge.target),
    ) else {
        return;
    };
    let progress = app.playback_progress();
    let marker = Point {
        x: source.x + (target.x - source.x) * progress,
        y: source.y + (target.y - source.y) * progress,
    };
    let (x, y) = canvas_to_cell(marker, area);
    render_overlay(
        frame,
        Rect::new(x, y, 1, 1),
        "◇",
        Style::new()
            .fg(palette.hot)
            .bg(palette.background)
            .add_modifier(Modifier::BOLD),
    );
}

fn render_flow_direction_markers(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    let edge_ids: Vec<_> = app.current_flow().map_or_else(
        || {
            let selected = selected_node(app).map(|node| node.id.as_str());
            app.graph
                .edges
                .iter()
                .filter(|edge| selected.is_some_and(|id| edge.source == id || edge.target == id))
                .map(|edge| edge.id.as_str())
                .collect()
        },
        |flow| flow.edge_ids.iter().map(String::as_str).collect(),
    );
    for (hop, edge_id) in edge_ids.iter().enumerate() {
        let Some(edge) = app.graph.edges.iter().find(|edge| edge.id == *edge_id) else {
            continue;
        };
        let (Some(source), Some(target)) = (
            app.layout.position(&edge.source),
            app.layout.position(&edge.target),
        ) else {
            continue;
        };
        let marker = Point {
            x: source.x + (target.x - source.x) * 0.55,
            y: source.y + (target.y - source.y) * 0.55,
        };
        let (x, y) = canvas_to_cell(marker, area);
        let active = app.flow_hop == Some(hop) && app.current_flow().is_some();
        render_overlay(
            frame,
            Rect::new(x, y, 1, 1),
            if is_fixture(app) {
                if active { "▶" } else { "›" }
            } else {
                direction_glyph(source, target)
            },
            Style::new()
                .fg(if active { palette.hot } else { palette.primary })
                .bg(palette.background)
                .add_modifier(Modifier::BOLD),
        );
    }
}

fn render_project_direction_markers(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    palette: Palette,
    plan: &MapRenderPlan,
) {
    let focused_edge_id = app
        .selected_relationship_edge()
        .map(|(_, _, edge, _)| edge.id.as_str());
    for edge in app
        .graph
        .edges
        .iter()
        .filter(|edge| plan.visible_edge_ids.contains(edge.id.as_str()))
    {
        let (Some(source), Some(target)) = (
            app.layout.position(&edge.source),
            app.layout.position(&edge.target),
        ) else {
            continue;
        };
        let marker = Point {
            x: source.x + (target.x - source.x) * 0.55,
            y: source.y + (target.y - source.y) * 0.55,
        };
        let (x, y) = canvas_to_cell(marker, area);
        let hop = app
            .current_flow()
            .and_then(|flow| flow.edge_ids.iter().position(|id| id == &edge.id));
        let active = hop.is_some_and(|hop| app.flow_hop == Some(hop));
        let color = match app.map_mode {
            MapMode::Overview => palette.secondary,
            MapMode::Focus if focused_edge_id == Some(edge.id.as_str()) => palette.hot,
            MapMode::Focus if edge.provenance == Provenance::Inferred => palette.inferred,
            MapMode::Focus => palette.primary,
            MapMode::Trace if active => palette.hot,
            MapMode::Trace
                if app
                    .flow_hop
                    .is_some_and(|active_hop| hop.is_some_and(|hop| hop < active_hop)) =>
            {
                palette.primary
            }
            MapMode::Trace => palette.secondary,
        };
        render_overlay(
            frame,
            Rect::new(x, y, 1, 1).intersection(area),
            direction_glyph(source, target),
            Style::new()
                .fg(color)
                .bg(palette.background)
                .add_modifier(Modifier::BOLD),
        );
    }
}

fn render_group_link_labels(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    palette: Palette,
    plan: &MapRenderPlan,
) {
    if plan.group_links.len() > 4 {
        render_group_link_legend(frame, area, palette, plan);
        return;
    }
    let mut detached_row = 0_u16;
    for link in &plan.group_links {
        let Some((source, target)) = group_link_points(app, plan, link) else {
            continue;
        };
        if group_link_crosses_unrelated_group(app, plan, link) {
            let inferred = if link.inferred == 0 {
                String::new()
            } else {
                format!(" ?{:02}", link.inferred)
            };
            let label = format!(
                "{}→{} ×{:02}{inferred}",
                truncate(&link.source_group.to_uppercase(), 8),
                truncate(&link.target_group.to_uppercase(), 8),
                link.total,
            );
            let y = area.bottom().saturating_sub(3 + detached_row);
            if y <= area.y {
                continue;
            }
            let width = (label.width() as u16).min(area.width.saturating_sub(4));
            render_overlay(
                frame,
                Rect::new(area.x + 2, y, width, 1).intersection(area),
                &label,
                Style::new()
                    .fg(if link.inferred == link.total {
                        palette.inferred
                    } else {
                        palette.secondary
                    })
                    .bg(palette.background)
                    .add_modifier(Modifier::BOLD),
            );
            detached_row = detached_row.saturating_add(1);
            continue;
        }
        let marker = Point {
            x: source.x + (target.x - source.x) * 0.55,
            y: source.y + (target.y - source.y) * 0.55,
        };
        let label = if link.inferred == 0 {
            format!("{}×{:02}", direction_glyph(source, target), link.total)
        } else {
            format!(
                "{}×{:02}?{:02}",
                direction_glyph(source, target),
                link.total,
                link.inferred
            )
        };
        let (x, y) = canvas_to_cell(marker, area);
        let width = (label.width() as u16).min(area.right().saturating_sub(x));
        render_overlay(
            frame,
            Rect::new(x, y, width, 1).intersection(area),
            &label,
            Style::new()
                .fg(if link.inferred == link.total {
                    palette.inferred
                } else {
                    palette.secondary
                })
                .bg(palette.background)
                .add_modifier(Modifier::BOLD),
        );
    }
}

fn render_group_link_legend(
    frame: &mut Frame<'_>,
    area: Rect,
    palette: Palette,
    plan: &MapRenderPlan,
) {
    let columns = if area.width >= 56 { 2 } else { 1 };
    let column_width = area.width / columns;
    let capacity = usize::from(area.height.saturating_sub(4)) * usize::from(columns);
    if capacity == 0 {
        return;
    }
    let visible = if plan.group_links.len() > capacity {
        capacity.saturating_sub(1)
    } else {
        plan.group_links.len()
    };
    for (index, link) in plan.group_links.iter().take(visible).enumerate() {
        let column = index as u16 % columns;
        let row = index as u16 / columns;
        let y = area.bottom().saturating_sub(4 + row);
        let x = area.x + column * column_width + 2;
        let available = column_width.saturating_sub(3);
        let inferred = if link.inferred == 0 {
            String::new()
        } else {
            format!(" ?{:02}", link.inferred)
        };
        let label = format!(
            "{}→{} ×{:02}{inferred}",
            truncate(&link.source_group.to_uppercase(), 8),
            truncate(&link.target_group.to_uppercase(), 8),
            link.total,
        );
        render_overlay(
            frame,
            Rect::new(x, y, available.min(label.width() as u16), 1).intersection(area),
            &truncate(&label, usize::from(available)),
            Style::new()
                .fg(if link.inferred == link.total {
                    palette.inferred
                } else {
                    palette.secondary
                })
                .bg(palette.background)
                .add_modifier(Modifier::BOLD),
        );
    }
    let hidden = plan.group_links.len().saturating_sub(visible);
    if hidden > 0 {
        let column = visible as u16 % columns;
        let row = visible as u16 / columns;
        let y = area.bottom().saturating_sub(4 + row);
        let x = area.x + column * column_width + 2;
        let available = column_width.saturating_sub(3);
        let label = format!("+{hidden:02} MORE");
        render_overlay(
            frame,
            Rect::new(x, y, available.min(label.width() as u16), 1).intersection(area),
            &truncate(&label, usize::from(available)),
            Style::new()
                .fg(palette.warning)
                .bg(palette.background)
                .add_modifier(Modifier::BOLD),
        );
    }
}

fn group_link_crosses_unrelated_group(
    app: &App,
    plan: &MapRenderPlan,
    link: &GroupLinkSummary,
) -> bool {
    let Some((source, target)) = group_link_points(app, plan, link) else {
        return false;
    };
    app.layout.groups().iter().any(|group| {
        group.label != link.source_group
            && group.label != link.target_group
            && segment_enters_group(source, target, group)
    })
}

fn segment_enters_group(
    source: Point,
    target: Point,
    group: &crate::graph_layout::LayoutGroup,
) -> bool {
    const CLEARANCE: f64 = 0.25;
    let min_x = group.x + CLEARANCE;
    let max_x = group.x + group.width - CLEARANCE;
    let min_y = group.y + CLEARANCE;
    let max_y = group.y + group.height - CLEARANCE;
    if min_x >= max_x || min_y >= max_y {
        return false;
    }

    let mut start = 0.0_f64;
    let mut end = 1.0_f64;
    for (origin, delta, minimum, maximum) in [
        (source.x, target.x - source.x, min_x, max_x),
        (source.y, target.y - source.y, min_y, max_y),
    ] {
        if delta.abs() <= f64::EPSILON {
            if origin <= minimum || origin >= maximum {
                return false;
            }
            continue;
        }
        let first = (minimum - origin) / delta;
        let second = (maximum - origin) / delta;
        let (near, far) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        start = start.max(near);
        end = end.min(far);
        if start >= end {
            return false;
        }
    }
    end > 0.0 && start < 1.0
}

fn group_link_points(
    app: &App,
    plan: &MapRenderPlan,
    link: &GroupLinkSummary,
) -> Option<(Point, Point)> {
    let source_group = app
        .layout
        .groups()
        .iter()
        .find(|group| group.label == link.source_group)?;
    let target_group = app
        .layout
        .groups()
        .iter()
        .find(|group| group.label == link.target_group)?;
    let source_center = Point {
        x: source_group.x + source_group.width / 2.0,
        y: source_group.y + source_group.height / 2.0,
    };
    let target_center = Point {
        x: target_group.x + target_group.width / 2.0,
        y: target_group.y + target_group.height / 2.0,
    };
    let dx = target_center.x - source_center.x;
    let dy = target_center.y - source_center.y;
    let distance = dx.hypot(dy);
    if distance <= f64::EPSILON {
        return None;
    }
    let boundary_scale = |half_width: f64, half_height: f64| {
        let x_scale = if dx.abs() <= f64::EPSILON {
            f64::INFINITY
        } else {
            half_width / dx.abs()
        };
        let y_scale = if dy.abs() <= f64::EPSILON {
            f64::INFINITY
        } else {
            half_height / dy.abs()
        };
        x_scale.min(y_scale)
    };
    let source_scale = boundary_scale(source_group.width / 2.0, source_group.height / 2.0);
    let target_scale = boundary_scale(target_group.width / 2.0, target_group.height / 2.0);
    let has_reverse = plan.group_links.iter().any(|candidate| {
        candidate.source_group == link.target_group && candidate.target_group == link.source_group
    });
    // The source-to-target vector already reverses for the return summary, so
    // the same positive perpendicular offset places the two directions on
    // opposite sides of the center line.
    let lane = if has_reverse { 3.5 } else { 0.0 };
    let lane_x = -dy / distance * lane;
    let lane_y = dx / distance * lane;
    Some((
        Point {
            x: source_center.x + dx * source_scale + lane_x,
            y: source_center.y + dy * source_scale + lane_y,
        },
        Point {
            x: target_center.x - dx * target_scale + lane_x,
            y: target_center.y - dy * target_scale + lane_y,
        },
    ))
}

fn direction_glyph(source: Point, target: Point) -> &'static str {
    let horizontal = target.x - source.x;
    let vertical = target.y - source.y;
    if horizontal.abs() >= vertical.abs() {
        if horizontal >= 0.0 { "→" } else { "←" }
    } else if vertical >= 0.0 {
        "↑"
    } else {
        "↓"
    }
}

fn render_group_labels(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    palette: Palette,
    plan: Option<&MapRenderPlan>,
) {
    for group in app.layout.groups() {
        let label = if is_fixture(app) {
            group.label.clone()
        } else {
            let internal = plan
                .filter(|_| app.map_mode == MapMode::Overview)
                .and_then(|plan| plan.internal_edge_counts.get(&group.label))
                .copied()
                .unwrap_or(0);
            let label = if internal == 0 {
                group.label.to_uppercase()
            } else {
                format!("{} ·{internal:02}", group.label.to_uppercase())
            };
            truncate(&label, 18)
        };
        let (x, y) = canvas_to_cell(
            Point {
                x: group.x + 1.0,
                y: group.y + group.height,
            },
            area,
        );
        render_overlay(
            frame,
            Rect::new(
                x,
                y,
                (label.width() as u16).min(area.right().saturating_sub(x)),
                1,
            ),
            &label,
            Style::new().fg(palette.secondary).bg(palette.background),
        );
    }
}

fn render_node_labels(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    palette: Palette,
    plan: Option<&MapRenderPlan>,
) {
    if !is_fixture(app) {
        if let Some(plan) = plan {
            render_project_node_labels(frame, area, app, palette, plan);
        }
        return;
    }
    const LABELED: [&str; 16] = [
        "N01", "N02", "N04", "N08", "N09", "N10", "N11", "N14", "N15", "N16", "N17", "N19", "N20",
        "N23", "N24", "N27",
    ];
    for node in app.graph.nodes.iter().filter(|node| {
        LABELED.contains(&node.id.as_str())
            || selected_node(app).is_some_and(|selected| selected.id == node.id)
    }) {
        let Some(point) = app.layout.position(&node.id) else {
            continue;
        };
        let (x, y) = canvas_to_cell(point, area);
        let selected = selected_node(app).is_some_and(|selected| node.id == selected.id)
            && !(app.playback == crate::PlaybackState::Playing
                && app.motion == crate::MotionMode::Full
                && app.animation_frame < crate::FRAMES_PER_HOP);
        let active_endpoint = selected_flow_edge(app)
            .is_some_and(|(_, _, edge, _, _)| edge.source == node.id || edge.target == node.id);
        let label = format!("{} {}", node_glyph(node), map_label(node));
        let max_width = area.right().saturating_sub(x).max(1);
        let width = (label.width() as u16).min(max_width);
        let style = if selected {
            Style::new()
                .fg(palette.background)
                .bg(palette.hot)
                .add_modifier(Modifier::BOLD)
        } else if matches!(node.kind, NodeKind::Unresolved) {
            Style::new().fg(palette.inferred).bg(palette.background)
        } else if active_endpoint {
            Style::new()
                .fg(palette.hot)
                .bg(palette.background)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(palette.text).bg(palette.background)
        };
        render_overlay(frame, Rect::new(x, y, width, 1), &label, style);
    }
}

fn render_project_node_labels(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    palette: Palette,
    plan: &MapRenderPlan,
) {
    let selected_id = selected_node(app).map(|node| node.id.as_str());
    let degree = |node: &Node| {
        app.graph
            .edges
            .iter()
            .filter(|edge| edge.source == node.id || edge.target == node.id)
            .count()
    };
    let mut nodes: Vec<_> = app.graph.nodes.iter().collect();
    nodes.sort_by(|left, right| {
        let priority = |node: &Node| {
            if Some(node.id.as_str()) == selected_id {
                0
            } else if matches!(node.kind, NodeKind::Entry) {
                1
            } else if matches!(node.kind, NodeKind::Unresolved) {
                2
            } else if matches!(node.kind, NodeKind::ExternalPackage) {
                3
            } else {
                4
            }
        };
        priority(left)
            .cmp(&priority(right))
            .then_with(|| degree(right).cmp(&degree(left)))
            .then_with(|| left.evidence.path.cmp(&right.evidence.path))
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut occupied = HashSet::new();
    for node in nodes
        .into_iter()
        .filter(|node| plan.visible_label_ids.contains(node.id.as_str()))
    {
        let Some(point) = app.layout.position(&node.id) else {
            continue;
        };
        let (point_x, y) = canvas_to_cell(point, area);
        let raw = format!("{} {}", node_glyph(node), project_map_label(node));
        let label = truncate(&raw, 19);
        let width = label.width() as u16;
        let right_x = point_x.saturating_add(1);
        let left_x = point_x.saturating_sub(width);
        let candidates = [right_x, left_x];
        let selected = Some(node.id.as_str()) == selected_id;
        let mut chosen = candidates.into_iter().find(|x| {
            *x >= area.x
                && x.saturating_add(width) <= area.right()
                && (0..width).all(|offset| !occupied.contains(&(x + offset, y)))
        });
        if chosen.is_none() && selected {
            chosen = Some(point_x.min(area.right().saturating_sub(width)).max(area.x));
        }
        let Some(x) = chosen else {
            continue;
        };
        let width = width.min(area.right().saturating_sub(x));
        if width == 0 {
            continue;
        }
        for offset in 0..width {
            occupied.insert((x + offset, y));
        }
        let style = if selected {
            Style::new()
                .fg(palette.hot)
                .bg(palette.panel)
                .add_modifier(Modifier::BOLD)
        } else if matches!(node.kind, NodeKind::Unresolved) {
            Style::new().fg(palette.inferred).bg(palette.background)
        } else if matches!(node.kind, NodeKind::Entry)
            || (app.map_mode == MapMode::Overview
                && plan.emphasized_node_ids.contains(node.id.as_str()))
        {
            Style::new()
                .fg(palette.hot)
                .bg(palette.background)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(palette.text).bg(palette.background)
        };
        render_overlay(frame, Rect::new(x, y, width, 1), &label, style);
    }
}

fn render_project_annotations(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette) {
    let mode = match app.map_mode {
        MapMode::Overview => "OVERVIEW // ×TOTAL ?INFERRED ·INTERNAL",
        MapMode::Focus => "FOCUS // 01-HOP STATIC IMPORTS",
        MapMode::Trace => "STATIC PATH",
    };
    let label = if app.layout.cycle_edges().is_empty() {
        format!("▷ {mode} // NOT RUNTIME DATA")
    } else {
        format!(
            "▷ {mode} // {} CYCLE EDGES // NOT RUNTIME DATA",
            app.layout.cycle_edges().len()
        )
    };
    let (x, y) = canvas_to_cell(Point { x: 2.0, y: 1.0 }, area);
    render_overlay(
        frame,
        Rect::new(x, y, (label.width() as u16).min(area.width), 1),
        &label,
        Style::new().fg(palette.warning).bg(palette.background),
    );
}

fn render_map_annotations(frame: &mut Frame<'_>, area: Rect, palette: Palette) {
    let (cycle_x, cycle_y) = canvas_to_cell(Point { x: 54.0, y: 49.0 }, area);
    render_overlay(
        frame,
        Rect::new(cycle_x, cycle_y, 10, 1),
        "↔ CYCLE 01",
        Style::new().fg(palette.warning).bg(palette.background),
    );
    let (static_x, static_y) = canvas_to_cell(Point { x: 2.0, y: 1.0 }, area);
    let (inferred_x, inferred_y) = canvas_to_cell(Point { x: 58.0, y: 17.0 }, area);
    render_overlay(
        frame,
        Rect::new(inferred_x, inferred_y, 15, 1),
        "◇? INFERRED .42",
        Style::new().fg(palette.inferred).bg(palette.background),
    );
    const STATIC_LABEL: &str = "▷ STATIC PATH // POSSIBLE STRUCTURAL ROUTE";
    render_overlay(
        frame,
        Rect::new(
            static_x,
            static_y,
            (STATIC_LABEL.width() as u16).min(area.width),
            1,
        ),
        STATIC_LABEL,
        Style::new().fg(palette.warning).bg(palette.background),
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App, palette: Palette, wide: bool) {
    let panel = match app.active_panel {
        crate::Panel::Map => "MAP",
        crate::Panel::Navigator => "NAV",
        crate::Panel::Inspector => "INSPECT",
    };
    let transport = if app.motion == crate::MotionMode::Off {
        "DISABLED"
    } else if app.playback == crate::PlaybackState::Playing {
        "PAUSE"
    } else {
        "PLAY"
    };
    let middle = if app.current_flow().is_none() {
        let mode = if is_fixture(app) {
            ""
        } else {
            match app.map_mode {
                MapMode::Overview => "  OVERVIEW",
                MapMode::Focus => "  FOCUS",
                MapMode::Trace => "",
            }
        };
        if wide {
            format!(
                "←→ NODE{mode}  F PATH  ,. EDGE  E EVIDENCE  TAB {panel}  M {}  ",
                app.motion.label()
            )
        } else {
            format!(
                "←→ NODE{mode}  F PATH  ,. EDGE  E DRAWER  TAB {panel}  M {}  ",
                app.motion.label()
            )
        }
    } else if wide {
        format!(
            "←→ NODE  F {}  ,. STEP  SPACE {transport}  M {}  E EVIDENCE  TAB {panel}  ",
            if is_fixture(app) { "FLOW" } else { "CLEAR" },
            app.motion.label(),
        )
    } else {
        format!(
            "←→ NODE  F {}  ,. STEP  SPACE {transport}  M {}  E DRAWER  ",
            if is_fixture(app) { "FLOW" } else { "CLEAR" },
            app.motion.label(),
        )
    };
    let return_hint = if app.active_panel != crate::Panel::Map {
        Some("MAP  ")
    } else if is_fixture(app) && app.current_flow().is_some() && app.flow_hop.is_some() {
        Some("CLEAR  ")
    } else if !is_fixture(app) && app.map_mode != MapMode::Overview {
        Some("BACK  ")
    } else {
        None
    };
    let mut commands = vec![Span::styled(middle, Style::new().fg(palette.text))];
    if let Some(label) = return_hint {
        commands.push(key("ESC", palette));
        commands.push(Span::styled(label, Style::new().fg(palette.text)));
    }
    commands.extend([
        key("T", palette),
        Span::styled(" THEME  ", Style::new().fg(palette.text)),
        key("Q", palette),
        Span::styled(" EXIT", Style::new().fg(palette.text)),
    ]);
    frame.render_widget(
        Paragraph::new(TextLine::from(commands)).block(instrument_block(
            "COMMAND",
            palette.primary,
            palette.panel,
        )),
        area,
    );
}

fn render_minimum_size(frame: &mut Frame<'_>, palette: Palette) {
    let area = centered_rect(
        62.min(frame.area().width),
        9.min(frame.area().height),
        frame.area(),
    );
    let message = vec![
        TextLine::styled(
            "WIREGLYPH // DISPLAY LIMIT",
            Style::new()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
        TextLine::from(""),
        TextLine::styled(
            format!(
                "CURRENT  {:03} × {:02}",
                frame.area().width,
                frame.area().height
            ),
            Style::new().fg(palette.text),
        ),
        TextLine::styled(
            format!("REQUIRED {MIN_WIDTH:03} × {MIN_HEIGHT:02}"),
            Style::new().fg(palette.hot),
        ),
        TextLine::from(""),
        TextLine::styled("ENLARGE TERMINAL // Q EXIT", Style::new().fg(palette.muted)),
    ];
    frame.render_widget(
        Paragraph::new(message).block(instrument_block(
            "FIELD DISPLAY",
            palette.primary,
            palette.panel,
        )),
        area,
    );
}

fn node_glyph(node: &Node) -> &'static str {
    match node.kind {
        NodeKind::Entry => "◆",
        NodeKind::Route => "▷",
        NodeKind::Adapter => "▤",
        NodeKind::Configuration => "⌬",
        NodeKind::ExternalPackage | NodeKind::ExternalService | NodeKind::ExternalSystem => "○",
        NodeKind::Unresolved => "◇?",
        _ => "□",
    }
}

fn map_label(node: &Node) -> &str {
    match node.id.as_str() {
        "N01" => "SERVER",
        "N02" => "APP.FACT",
        "N04" => "WEB.ENTRY",
        "N08" => "API.CLIENT",
        "N09" => "API.ROUTER",
        "N10" => "GET/:ID",
        "N11" => "GET.SYSTEM",
        "N14" => "SYSTEM.SVC",
        "N15" => "SYS.REPO",
        "N16" => "FLOW",
        "N17" => "GRAPH",
        "N19" => "SQLITE",
        "N20" => "DB.CONN",
        "N23" => "SQLITE3",
        "N24" => "REPO.PROBE",
        "N27" => "ALERT",
        _ => node.label.as_str(),
    }
}

fn project_map_label(node: &Node) -> &str {
    node.label.rsplit('/').next().unwrap_or(&node.label)
}

fn truncate(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let content_width = width - 1;
    let mut used = 0;
    let mut truncated = String::new();
    for grapheme in value.graphemes(true) {
        let grapheme_width = grapheme.width();
        if used + grapheme_width > content_width {
            break;
        }
        used += grapheme_width;
        truncated.push_str(grapheme);
    }
    truncated.push('…');
    truncated
}

fn middle_truncate(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".into();
    }
    let graphemes: Vec<_> = value.graphemes(true).collect();
    let content_width = width - 1;
    let left_budget = content_width.div_ceil(2);
    let right_budget = content_width / 2;
    let mut prefix = String::new();
    let mut prefix_width = 0;
    let mut prefix_count = 0;
    for grapheme in &graphemes {
        let grapheme_width = grapheme.width();
        if prefix_width + grapheme_width > left_budget {
            break;
        }
        prefix.push_str(grapheme);
        prefix_width += grapheme_width;
        prefix_count += 1;
    }
    let mut suffix = Vec::new();
    let mut suffix_width = 0;
    for grapheme in graphemes.iter().skip(prefix_count).rev() {
        let grapheme_width = grapheme.width();
        if suffix_width + grapheme_width > right_budget {
            break;
        }
        suffix.push(*grapheme);
        suffix_width += grapheme_width;
    }
    suffix.reverse();
    format!("{prefix}…{}", suffix.concat())
}

fn dashed_points(source: Point, target: Point) -> Vec<(f64, f64)> {
    (0..=12)
        .filter(|step| step % 2 == 0)
        .map(|step| {
            let t = f64::from(step) / 12.0;
            (
                source.x + (target.x - source.x) * t,
                source.y + (target.y - source.y) * t,
            )
        })
        .collect()
}

fn key(label: &'static str, palette: Palette) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::new()
            .fg(palette.background)
            .bg(palette.primary)
            .add_modifier(Modifier::BOLD),
    )
}

fn render_overlay(frame: &mut Frame<'_>, area: Rect, text: &str, style: Style) {
    if !area.is_empty() {
        frame.render_widget(Paragraph::new(text.to_owned()).style(style), area);
    }
}

fn inner_rect(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn instrument_block(title: &'static str, border: Color, panel: Color) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(Style::new().fg(border))
        .style(Style::new().bg(panel))
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn inferred_edge_uses_a_sparse_dash_pattern() {
        let points = dashed_points(Point { x: 0.0, y: 0.0 }, Point { x: 12.0, y: 6.0 });
        assert_eq!(points.len(), 7);
        assert_eq!(points.first(), Some(&(0.0, 0.0)));
        assert_eq!(points.last(), Some(&(12.0, 6.0)));
    }

    #[test]
    fn truncation_respects_terminal_cell_width_and_zero_width() {
        assert_eq!(truncate("repository", 0), "");
        assert_eq!(truncate("資料視覚", 5), "資料…");
        assert_eq!(truncate("資料視覚", 5).width(), 5);
        assert_eq!(truncate("資料", 4), "資料");
        assert_eq!(truncate("e\u{301}clair", 2), "e\u{301}…");
        assert_eq!(truncate("👩‍💻tools", 3), "👩‍💻…");
        assert_eq!(
            middle_truncate("src/very/long/module.ts", 12),
            "src/ve…le.ts"
        );
        let middle = middle_truncate("資料/長い/経路.py", 9);
        assert!(middle.width() <= 9);
        assert!(middle.contains('…'));
    }

    #[test]
    fn partial_scan_health_is_visible_in_status_and_navigation() {
        let mut graph = crate::fixture::load_beacon_ops().expect("fixture should load");
        graph.repository = "PARTIAL PROJECT".into();
        graph.scan_summary.source = "local_static_scan".into();
        graph.scan_summary.files_discovered = 5;
        graph.scan_summary.files_scanned = 3;
        graph.scan_summary.files_skipped = 1;
        graph.scan_summary.parse_warnings = 1;
        graph.flows.clear();
        let app = App::from_graph(graph);

        let screen = render_for_test(&app, 140, 40);
        assert!(screen.contains("PARTIAL  OVERVIEW"), "{screen}");
        assert!(screen.contains("SCAN PARTIAL"), "{screen}");
        assert!(screen.contains("03/05 FILES"), "{screen}");
        assert!(screen.contains("S01 P01 T00"), "{screen}");
    }

    #[test]
    fn navigator_windows_forty_groups_around_the_selection_at_both_sizes() {
        let fixture = crate::fixture::load_beacon_ops().expect("fixture should load");
        let template = fixture.nodes[0].clone();
        let mut graph = fixture;
        graph.repository = "FORTY GROUPS".into();
        graph.scan_summary.source = "local_static_scan".into();
        graph.scan_summary.files_discovered = 40;
        graph.scan_summary.files_scanned = 40;
        graph.nodes = (0..40)
            .map(|index| {
                let mut node = template.clone();
                node.id = format!("N{index:02}");
                node.group = format!("GROUP-{index:02}");
                node.label = format!("module-{index:02}");
                node.kind = NodeKind::Module;
                node.evidence.path = format!("group-{index:02}/module.ts");
                node
            })
            .collect();
        graph.edges.clear();
        graph.flows.clear();
        let mut app = App::from_graph(graph);
        app.selected_node = Some(20);
        app.active_panel = crate::Panel::Navigator;

        for (width, height) in [(100, 30), (140, 40)] {
            let screen = render_for_test(&app, width, height);
            assert!(screen.contains("GROUP-20"), "{screen}");
            assert!(screen.contains("ABOVE"), "{screen}");
            assert!(screen.contains("BELOW"), "{screen}");
            assert!(screen.contains("SCAN SCOPED OK"), "{screen}");
            assert!(screen.contains("NOT RUNTIME DATA"), "{screen}");
        }
    }

    #[test]
    fn fifth_outgoing_relationship_has_bounded_evidence_at_both_sizes() {
        let mut graph = crate::fixture::load_beacon_ops().expect("fixture should load");
        graph.repository = "HIGH DEGREE".into();
        graph.scan_summary.source = "local_static_scan".into();
        graph.flows.clear();
        let mut app = App::from_graph(graph);
        app.selected_node = app.graph.nodes.iter().position(|node| node.id == "N02");
        for _ in 0..5 {
            app.step_relationship(1);
        }
        let selected = app
            .selected_relationship_edge()
            .expect("fifth relationship should exist")
            .2
            .id
            .clone();

        for (width, height) in [(100, 30), (140, 40)] {
            let screen = render_for_test(&app, width, height);
            assert!(screen.contains("RELATIONSHIP 05/05"), "{screen}");
            assert!(screen.contains(&selected), "{screen}");
            assert!(screen.contains("STATIC IMPORT RELATIONSHIP"), "{screen}");
            assert!(screen.contains("NOT OBSERVED RUNTIME DATA"), "{screen}");
            assert!(screen.contains("[,] PREV   [.] NEXT"), "{screen}");
        }
    }

    #[test]
    fn generic_edge_markers_follow_actual_geometry() {
        let origin = Point { x: 5.0, y: 5.0 };
        assert_eq!(direction_glyph(origin, Point { x: 8.0, y: 5.0 }), "→");
        assert_eq!(direction_glyph(origin, Point { x: 2.0, y: 5.0 }), "←");
        assert_eq!(direction_glyph(origin, Point { x: 5.0, y: 8.0 }), "↑");
        assert_eq!(direction_glyph(origin, Point { x: 5.0, y: 2.0 }), "↓");
    }

    #[test]
    fn reciprocal_overview_links_use_separate_readable_lanes() {
        let mut graph = progressive_disclosure_graph();
        graph.nodes.truncate(2);
        graph.nodes[0].group = "LEFT".into();
        graph.nodes[1].group = "RIGHT".into();
        graph.edges = vec![
            graph.edges[0].clone(),
            crate::model::Edge {
                id: "reverse".into(),
                source: graph.edges[0].target.clone(),
                target: graph.edges[0].source.clone(),
                ..graph.edges[0].clone()
            },
        ];
        graph.flows.clear();
        let app = App::from_graph(graph);
        let plan = build_render_plan(&app.graph, MapMode::Overview, None, None);
        let forward = plan
            .group_links
            .iter()
            .find(|link| link.source_group == "LEFT")
            .expect("forward summary should exist");
        let reverse = plan
            .group_links
            .iter()
            .find(|link| link.source_group == "RIGHT")
            .expect("reverse summary should exist");
        let (forward_source, forward_target) =
            group_link_points(&app, &plan, forward).expect("forward lane should resolve");
        let (reverse_source, reverse_target) =
            group_link_points(&app, &plan, reverse).expect("reverse lane should resolve");
        let forward_midpoint = Point {
            x: forward_source.x + (forward_target.x - forward_source.x) * 0.55,
            y: forward_source.y + (forward_target.y - forward_source.y) * 0.55,
        };
        let reverse_midpoint = Point {
            x: reverse_source.x + (reverse_target.x - reverse_source.x) * 0.55,
            y: reverse_source.y + (reverse_target.y - reverse_source.y) * 0.55,
        };

        assert!(
            (forward_midpoint.y - reverse_midpoint.y).abs() >= 6.0,
            "reciprocal summary labels need distinct terminal rows"
        );
    }

    #[test]
    fn nonadjacent_overview_link_is_named_instead_of_crossing_an_unrelated_group() {
        let mut graph = progressive_disclosure_graph();
        graph.nodes.truncate(3);
        graph.nodes[0].group = "SRC".into();
        graph.nodes[1].group = "TOOLS".into();
        graph.nodes[2].group = "EXTERNAL".into();
        graph.nodes[2].kind = NodeKind::ExternalPackage;
        graph.edges.truncate(2);
        graph.edges.push(crate::model::Edge {
            id: "tools-external".into(),
            source: graph.nodes[1].id.clone(),
            target: graph.nodes[2].id.clone(),
            ..graph.edges[0].clone()
        });
        graph.flows.clear();
        let mut app = App::from_graph(graph);
        app.active_panel = crate::Panel::Map;
        let plan = build_render_plan(&app.graph, MapMode::Overview, None, None);
        let long_link = plan
            .group_links
            .iter()
            .find(|link| link.source_group == "SRC" && link.target_group == "EXTERNAL")
            .expect("long summary should exist");
        let adjacent_link = plan
            .group_links
            .iter()
            .find(|link| link.source_group == "SRC" && link.target_group == "TOOLS")
            .expect("adjacent summary should exist");

        assert!(group_link_crosses_unrelated_group(&app, &plan, long_link));
        assert!(!group_link_crosses_unrelated_group(
            &app,
            &plan,
            adjacent_link
        ));
        for (width, height) in [(100, 30), (140, 40)] {
            let screen = render_for_test(&app, width, height);
            assert!(screen.contains("SRC→EXTERNAL ×01"), "{screen}");
            assert!(screen.contains("×TOTAL ?INFERRED ·INTERNAL"), "{screen}");
        }
    }

    #[test]
    fn dense_overview_uses_a_readable_named_group_link_legend() {
        let mut graph = progressive_disclosure_graph();
        let template = graph.nodes[0].clone();
        graph.nodes = (0..6)
            .map(|index| Node {
                id: format!("N{index}"),
                group: format!("G{index}"),
                label: format!("NODE{index}"),
                evidence: crate::model::Evidence {
                    path: format!("g{index}/node.rs"),
                    ..template.evidence.clone()
                },
                ..template.clone()
            })
            .collect();
        let edge_template = graph.edges[0].clone();
        graph.edges = (1..6)
            .map(|index| crate::model::Edge {
                id: format!("E{index}"),
                source: "N0".into(),
                target: format!("N{index}"),
                evidence: crate::model::Evidence {
                    path: "g0/node.rs".into(),
                    line_start: index,
                    line_end: index,
                },
                ..edge_template.clone()
            })
            .collect();
        graph.flows.clear();
        let app = App::from_graph(graph);

        let screen = render_for_test(&app, 140, 40);
        for target in 1..6 {
            assert!(screen.contains(&format!("G0→G{target} ×01")), "{screen}");
        }
    }

    #[test]
    fn overflowing_group_link_legend_reports_every_hidden_summary() {
        let mut graph = progressive_disclosure_graph();
        let node_template = graph.nodes[0].clone();
        graph.nodes = (0..40)
            .map(|index| Node {
                id: format!("N{index}"),
                group: format!("G{index}"),
                label: format!("NODE{index}"),
                evidence: crate::model::Evidence {
                    path: format!("g{index}/node.rs"),
                    ..node_template.evidence.clone()
                },
                ..node_template.clone()
            })
            .collect();
        let edge_template = graph.edges[0].clone();
        graph.edges = (0..2)
            .flat_map(|source| (2..40).map(move |target| (source, target)))
            .map(|(source, target)| crate::model::Edge {
                id: format!("E{source}-{target}"),
                source: format!("N{source}"),
                target: format!("N{target}"),
                evidence: crate::model::Evidence {
                    path: format!("g{source}/node.rs"),
                    line_start: target,
                    line_end: target,
                },
                ..edge_template.clone()
            })
            .collect();
        graph.flows.clear();
        let app = App::from_graph(graph);

        let screen = render_for_test(&app, 100, 30);
        assert!(screen.contains(" MORE"), "{screen}");
    }

    #[test]
    fn focus_hides_unrelated_labels_and_direction_markers_at_both_sizes() {
        for (width, height) in [(100, 30), (140, 40)] {
            let mut overview = App::from_graph(progressive_disclosure_graph());
            overview.active_panel = crate::Panel::Map;
            let overview_screen = render_for_test(&overview, width, height);

            let selected = overview
                .graph
                .nodes
                .iter()
                .position(|node| node.id == "selected")
                .expect("selected test node should exist");
            overview.selected_node = Some(selected);
            overview.map_mode = MapMode::Focus;
            let focus_screen = render_for_test(&overview, width, height);

            assert!(focus_screen.contains("SELECTED"), "{focus_screen}");
            assert!(focus_screen.contains("NEIGHBOR"), "{focus_screen}");
            assert!(!focus_screen.contains("UNRELATED-HIDDEN"), "{focus_screen}");
            assert_eq!(
                map_direction_marker_count(&focus_screen, width),
                map_direction_marker_count(&overview_screen, width) + 1,
                "focus should add only the selected incident edge marker at {width}x{height}:\n{focus_screen}"
            );
        }
    }

    #[test]
    fn scanned_map_modes_use_truthful_copy_at_both_sizes() {
        for (width, height) in [(100, 30), (140, 40)] {
            let mut app = App::from_graph(progressive_disclosure_graph());
            app.active_panel = crate::Panel::Map;

            let overview = render_for_test(&app, width, height);
            assert!(overview.contains("SYSTEM MAP // OVERVIEW"), "{overview}");
            assert!(!overview.contains("ESC BACK"), "{overview}");
            assert!(
                overview.contains("OVERVIEW // ×TOTAL ?INFERRED ·INTERNAL"),
                "{overview}"
            );
            if width == 140 {
                assert!(overview.contains("ROOT CANDIDATES"), "{overview}");
            }

            app.map_mode = MapMode::Focus;
            app.selected_node = app
                .graph
                .nodes
                .iter()
                .position(|node| node.id == "selected");
            let focus = render_for_test(&app, width, height);
            assert!(focus.contains("SYSTEM MAP // FOCUS"), "{focus}");
            assert!(focus.contains("FOCUS // 01-HOP STATIC IMPORTS"), "{focus}");
            assert!(focus.contains("ESC BACK"), "{focus}");
            assert!(focus.contains("EXIT"), "{focus}");

            app.active_trace = app.graph.flows.first().cloned();
            app.flow_hop = Some(0);
            app.map_mode = MapMode::Trace;
            let trace = render_for_test(&app, width, height);
            assert!(trace.contains("SYSTEM MAP // STATIC PATH"), "{trace}");
            assert!(trace.contains("STATIC PATH  PAUSE  01/01"), "{trace}");
            assert!(trace.contains("ESC BACK"), "{trace}");
            assert!(trace.contains("EXIT"), "{trace}");

            app.active_panel = crate::Panel::Inspector;
            let drawer = render_for_test(&app, width, height);
            assert!(drawer.contains("ESC MAP"), "{drawer}");
            assert!(!drawer.contains("ESC BACK"), "{drawer}");
            assert!(drawer.contains("EXIT"), "{drawer}");
        }
    }

    #[test]
    fn fixture_active_flow_exposes_escape_clear_at_both_sizes() {
        let mut app = App::new().expect("fixture should load");
        assert!(app.select_static_flow_hop(0));
        app.active_panel = crate::Panel::Map;

        for (width, height) in [(100, 30), (140, 40)] {
            let screen = render_for_test(&app, width, height);
            assert!(screen.contains("ESC CLEAR"), "{screen}");
            assert!(!screen.contains("ESC BACK"), "{screen}");
            assert!(screen.contains("EXIT"), "{screen}");
        }

        app.active_trace = None;
        app.flow_hop = Some(usize::MAX);
        for (width, height) in [(100, 30), (140, 40)] {
            let stale = render_for_test(&app, width, height);
            assert!(!stale.contains("ESC CLEAR"), "{stale}");
        }
    }

    #[test]
    fn stale_selection_and_missing_flow_render_without_panicking() {
        let mut app = App::new().expect("fixture should load");
        app.selected_node = Some(usize::MAX);
        app.flow_hop = Some(usize::MAX);
        app.active_trace = None;
        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("empty selection state should render");

        let symbols = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(symbols.contains("NO NODE SELECTED"));
        assert!(symbols.contains("NO STATIC FLOW AVAILABLE"));
    }

    #[test]
    fn default_wide_inspector_identifies_the_application_entry() {
        let app = App::new().expect("fixture should load");
        let screen = render_for_test(&app, 140, 40);

        assert!(screen.contains("SERVER.ENTRY"), "{screen}");
        assert!(screen.contains("src/server.ts"), "{screen}");
        assert!(screen.contains("LINES 1–24"), "{screen}");
    }

    #[test]
    fn scanned_project_uses_dynamic_identity_and_truthful_static_language() {
        let root = std::env::temp_dir().join(format!(
            "wireglyph-ui-project-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"main":"src/main.ts","dependencies":{"zod":"1"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("src/main.ts"),
            "import './worker';\nimport { z } from 'zod';\n",
        )
        .unwrap();
        std::fs::write(root.join("src/worker.ts"), "export const worker = true;\n").unwrap();
        let graph = crate::scanner::scan_project(&root).unwrap();
        let mut app = App::from_graph(graph);
        app.active_panel = crate::Panel::Inspector;

        for (width, height) in [(100, 30), (140, 40)] {
            let screen = render_for_test(&app, width, height);
            assert!(screen.contains("wireglyph-ui-project"), "{screen}");
            assert!(screen.contains("SYSTEM MAP // OVERVIEW"), "{screen}");
            assert!(
                screen.contains("NOT RUNTIME DATA") || screen.contains("NOT OBSERVED RUNTIME DATA"),
                "{screen}"
            );
            assert!(screen.contains("src/main.ts"), "{screen}");
            assert!(screen.contains("F PATH"), "{screen}");
            if width == 140 {
                assert!(screen.contains("▷ OVERVIEW"), "{screen}");
            }
            assert!(!screen.contains("BEACON"), "{screen}");
            assert!(!screen.contains("INFERRED .42"), "{screen}");
        }
        assert!(app.arm_selected_trace());
        for (width, height) in [(100, 30), (140, 40)] {
            let screen = render_for_test(&app, width, height);
            assert!(screen.contains("STATIC PATH 01/01"), "{screen}");
            assert!(screen.contains("STATIC PATH"), "{screen}");
            assert!(screen.contains("F CLEAR"), "{screen}");
            assert!(screen.contains("NOT OBSERVED RUNTIME DATA"), "{screen}");
        }
        app.clear_trace_context();
        app.map_mode = MapMode::Focus;
        let worker = app
            .graph
            .nodes
            .iter()
            .position(|node| node.evidence.path == "src/worker.ts")
            .unwrap();
        app.selected_node = Some(worker);
        let screen = render_for_test(&app, 140, 40);
        assert!(screen.contains("NO OUTWARD IMPORTS"), "{screen}");
        assert!(screen.contains("SELECT ANOTHER MODULE"), "{screen}");
        app.graph.repository = "BEACON OPS".into();
        let screen = render_for_test(&app, 140, 40);
        assert!(screen.contains("SYSTEM MAP // FOCUS"), "{screen}");
        assert!(!screen.contains("SYSTEM DETAIL"), "{screen}");
        assert!(!screen.contains("INFERRED .42"), "{screen}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fifth_flow_hop_exposes_the_http_domain_boundary_at_both_sizes() {
        let mut app = App::new().expect("fixture should load");
        app.flow_hop = Some(4);
        app.active_panel = crate::Panel::Inspector;
        app.selected_node = Some(
            app.graph
                .nodes
                .iter()
                .position(|node| node.id == "N14")
                .expect("flow target should exist"),
        );

        for (width, height) in [(100, 30), (140, 40)] {
            let screen = render_for_test(&app, width, height);
            assert!(screen.contains("STATIC EDGE 05/09"), "{screen}");
            assert!(screen.contains("HTTP → DOMAIN"), "{screen}");
            assert!(screen.contains("E17 // CALLS"), "{screen}");
            assert!(
                screen.contains("src/http/handlers/get_system.ts"),
                "{screen}"
            );
            assert!(screen.contains("NOT OBSERVED RUNTIME DATA"), "{screen}");
            if width == 100 {
                assert!(
                    !screen.contains("DB.CONN"),
                    "compact drawer should clear the covered map: {screen}"
                );
            }
        }
    }

    #[test]
    fn every_static_hop_resolves_its_exact_evidence_without_panicking() {
        let mut app = App::new().expect("fixture should load");
        app.active_panel = crate::Panel::Inspector;
        let evidence = app.graph.flows[0]
            .edge_ids
            .iter()
            .map(|edge_id| {
                let edge = app
                    .graph
                    .edges
                    .iter()
                    .find(|edge| edge.id == *edge_id)
                    .expect("fixture contract resolves edge");
                (edge.id.clone(), edge.evidence.path.clone())
            })
            .collect::<Vec<_>>();

        for (hop, (edge_id, path)) in evidence.iter().enumerate() {
            assert!(app.select_static_flow_hop(hop));
            for (width, height) in [(100, 30), (140, 40)] {
                let screen = render_for_test(&app, width, height);
                assert!(
                    screen.contains(edge_id),
                    "missing {edge_id} at {width}x{height}"
                );
                assert!(screen.contains(path), "missing {path} at {width}x{height}");
            }
        }
    }

    #[test]
    fn compact_flow_evidence_matches_the_reviewed_golden() {
        let mut app = App::new().expect("fixture should load");
        assert!(app.select_static_flow_hop(4));

        insta::assert_snapshot!("compact_flow_evidence", render_for_test(&app, 100, 30));
    }

    #[test]
    fn playback_modes_render_truthful_status_and_motion_semantics() {
        let mut full = App::new().expect("fixture should load");
        assert!(full.start_static_playback());
        for _ in 0..54 {
            assert!(full.advance_static_playback());
        }
        let full_screen = render_for_test(&full, 100, 30);
        assert!(full_screen.contains("STATIC F01  PLAY  05/09"));
        assert!(full_screen.contains("SPACE PAUSE"));
        assert!(!full_screen.contains("INSPECTOR // FLOW EVIDENCE"));
        assert!(
            render_for_test(&full, 140, 40).contains("NOT OBSERVED RUNTIME DATA"),
            "wide playback must retain the static-analysis warning"
        );

        let mut reduced = App::new().expect("fixture should load");
        reduced.set_motion_mode(crate::MotionMode::Reduced);
        assert!(reduced.start_static_playback());
        for _ in 0..4 {
            assert!(reduced.advance_static_playback());
        }
        let reduced_screen = render_for_test(&reduced, 100, 30);
        assert!(reduced_screen.contains("STATIC F01  REDUCED  05/09"));
        assert!(
            full_screen.matches('◇').count() > reduced_screen.matches('◇').count(),
            "only full motion should add an interpolated marker"
        );

        let mut off = App::new().expect("fixture should load");
        off.set_motion_mode(crate::MotionMode::Off);
        assert!(off.select_static_flow_hop(4));
        let off_screen = render_for_test(&off, 100, 30);
        assert!(off_screen.contains("STATIC F01  MOTION OFF  05/09"));
        assert!(off_screen.contains("SPACE DISABLED"));
        assert!(off_screen.contains("INSPECTOR // FLOW EVIDENCE"));
    }

    fn render_for_test(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render(frame, app))
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

    fn map_direction_marker_count(screen: &str, width: u16) -> usize {
        screen
            .lines()
            .skip(3)
            .take(screen.lines().count().saturating_sub(6))
            .flat_map(|line| {
                if width >= WIDE_WIDTH {
                    line.chars().skip(22).take(width as usize - 56).collect()
                } else {
                    line.to_owned()
                }
                .chars()
                .collect::<Vec<_>>()
            })
            .filter(|character| matches!(character, '←' | '→' | '↑' | '↓'))
            .count()
    }

    fn progressive_disclosure_graph() -> crate::model::Graph {
        use crate::model::{Edge, Evidence, Flow, Graph, ScanSummary};

        let evidence = |path: &str| Evidence {
            path: path.into(),
            line_start: 1,
            line_end: 1,
        };
        let node = |id: &str, label: &str, path: &str| Node {
            id: id.into(),
            group: "CORE".into(),
            label: label.into(),
            kind: NodeKind::Module,
            evidence: evidence(path),
        };
        let edge = |id: &str, source: &str, target: &str| Edge {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            relationship: "imports".into(),
            provenance: Provenance::Extracted,
            confidence: 1.0,
            evidence: evidence(&format!("{source}.ts")),
            import_specifier: None,
        };
        Graph {
            schema_version: 2,
            repository: "PROGRESSIVE DISCLOSURE".into(),
            nodes: vec![
                node("neighbor", "NEIGHBOR", "a-neighbor.ts"),
                node("selected", "SELECTED", "b-selected.ts"),
                node("unrelated-a", "UNRELATED-HIDDEN-A", "c-unrelated.ts"),
                node("unrelated-b", "UNRELATED-HIDDEN-B", "d-unrelated.ts"),
            ],
            edges: vec![
                edge("incident", "neighbor", "selected"),
                edge("unrelated-entry", "neighbor", "unrelated-a"),
                edge("unrelated", "unrelated-a", "unrelated-b"),
            ],
            flows: vec![Flow {
                id: "static-path".into(),
                label: "STATIC PATH".into(),
                provenance: Provenance::Extracted,
                node_ids: vec!["neighbor".into(), "selected".into()],
                edge_ids: vec!["incident".into()],
            }],
            scan_summary: ScanSummary {
                source: "local_static_scan".into(),
                files_discovered: 4,
                files_scanned: 4,
                files_skipped: 0,
                skipped_by_reason: BTreeMap::new(),
                parse_warnings: 0,
                traversal_errors: 0,
                inferred_edges: 0,
            },
        }
    }
}

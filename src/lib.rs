pub mod cli;
pub mod fixture;
pub mod graph_layout;
pub mod map_view;
pub mod model;
pub mod path_explorer;
pub mod path_export;
pub mod scanner;
pub mod theme;
pub mod ui;

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::{
    fixture::load_beacon_ops,
    graph_layout::GraphLayout,
    map_view::{MapMode, root_candidate_ids},
    theme::ThemeName,
};

const FULL_MOTION_INTERVAL: Duration = Duration::from_millis(84);
const DISCRETE_MOTION_INTERVAL: Duration = Duration::from_secs(1);
const IDLE_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FRAMES_PER_HOP: u8 = 12;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Panel {
    #[default]
    Map,
    Navigator,
    Inspector,
}

impl Panel {
    const fn next(self) -> Self {
        match self {
            Self::Map => Self::Navigator,
            Self::Navigator => Self::Inspector,
            Self::Inspector => Self::Map,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PlaybackState {
    #[default]
    Paused,
    Playing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MotionMode {
    #[default]
    Full,
    Reduced,
    Off,
}

impl MotionMode {
    const fn next(self) -> Self {
        match self {
            Self::Full => Self::Reduced,
            Self::Reduced => Self::Off,
            Self::Off => Self::Full,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Reduced => "REDUCED",
            Self::Off => "OFF",
        }
    }
}

#[derive(Debug)]
pub struct App {
    graph: model::Graph,
    layout: GraphLayout,
    theme: ThemeName,
    selected_node: Option<usize>,
    active_trace: Option<model::Flow>,
    relationship_cursor: Option<usize>,
    active_panel: Panel,
    map_mode: MapMode,
    flow_hop: Option<usize>,
    playback: PlaybackState,
    motion: MotionMode,
    animation_frame: u8,
    completed: bool,
    next_tick: Option<Instant>,
    interrupted: Arc<AtomicBool>,
    should_quit: bool,
}

impl App {
    pub fn new() -> Result<Self, serde_json::Error> {
        Ok(Self::from_graph(load_beacon_ops()?))
    }

    pub fn from_graph(graph: model::Graph) -> Self {
        let layout = GraphLayout::for_graph(&graph);
        let is_fixture = graph.scan_summary.source == "synthetic_fixture";
        let active_trace = is_fixture.then(|| graph.flows.first().cloned()).flatten();
        let selected_node = graph
            .nodes
            .iter()
            .position(|node| matches!(node.kind, model::NodeKind::Entry))
            .or_else(|| {
                (!is_fixture)
                    .then(|| root_candidate_ids(&graph).into_iter().next())
                    .flatten()
                    .and_then(|root_id| graph.nodes.iter().position(|node| node.id == root_id))
            })
            .or_else(|| (!graph.nodes.is_empty()).then_some(0));
        Self {
            graph,
            layout,
            theme: ThemeName::AmberPlotter,
            selected_node,
            active_trace,
            relationship_cursor: None,
            active_panel: Panel::Map,
            map_mode: if is_fixture {
                MapMode::Trace
            } else {
                MapMode::Overview
            },
            flow_hop: None,
            playback: PlaybackState::Paused,
            motion: MotionMode::Full,
            animation_frame: 0,
            completed: false,
            next_tick: None,
            interrupted: Arc::new(AtomicBool::new(false)),
            should_quit: false,
        }
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        terminal.draw(|frame| ui::render(frame, &self))?;
        while !self.should_quit && !self.interrupted.load(Ordering::Relaxed) {
            let now = Instant::now();
            let changed = if let Some(wait) = self.time_until_tick(now) {
                if wait.is_zero() {
                    self.advance_playback(now)
                } else if event::poll(wait)? {
                    self.handle_event(event::read()?)
                } else {
                    self.advance_playback(Instant::now())
                }
            } else if event::poll(IDLE_EVENT_POLL_INTERVAL)? {
                self.handle_event(event::read()?)
            } else {
                false
            };
            if changed && !self.should_quit {
                terminal.draw(|frame| ui::render(frame, &self))?;
            }
        }
        Ok(())
    }

    pub fn set_theme(&mut self, theme: ThemeName) {
        self.theme = theme;
    }

    pub fn with_interrupt_flag(mut self, interrupted: Arc<AtomicBool>) -> Self {
        self.interrupted = interrupted;
        self
    }

    pub fn set_motion_mode(&mut self, motion: MotionMode) {
        self.motion = motion;
        self.pause_playback();
        self.animation_frame = 0;
    }

    pub fn start_static_playback(&mut self) -> bool {
        if self.motion == MotionMode::Off || self.active_trace.is_none() {
            return false;
        }
        if self.flow_hop.is_none() {
            if self.flow_hop.is_none() && !self.select_static_flow_hop(0) {
                return false;
            }
            self.active_panel = Panel::Map;
        } else if self.completed {
            let active_panel = self.active_panel;
            if !self.select_static_flow_hop(0) {
                return false;
            }
            self.active_panel = active_panel;
        }
        self.playback = PlaybackState::Playing;
        self.schedule_next_tick(Instant::now());
        true
    }

    pub fn advance_static_playback(&mut self) -> bool {
        self.advance_playback(Instant::now())
    }

    pub fn select_static_flow_hop(&mut self, hop: usize) -> bool {
        let Some(flow) = self.active_trace.as_ref() else {
            return false;
        };
        let Some(edge_id) = flow.edge_ids.get(hop) else {
            return false;
        };
        let Some(edge) = self.graph.edges.iter().find(|edge| edge.id == *edge_id) else {
            return false;
        };
        let (Some(source_id), Some(target_id)) =
            (flow.node_ids.get(hop), flow.node_ids.get(hop + 1))
        else {
            return false;
        };
        if edge.source != *source_id || edge.target != *target_id {
            return false;
        }
        if !self.graph.nodes.iter().any(|node| node.id == *source_id) {
            return false;
        }
        let Some(selected_node) = self
            .graph
            .nodes
            .iter()
            .position(|node| node.id == *target_id)
        else {
            return false;
        };
        self.flow_hop = Some(hop);
        self.relationship_cursor = None;
        self.selected_node = Some(selected_node);
        self.active_panel = Panel::Inspector;
        if !self.is_fixture() {
            self.map_mode = MapMode::Trace;
        }
        self.playback = PlaybackState::Paused;
        self.animation_frame = 0;
        self.completed = false;
        self.next_tick = None;
        true
    }

    /// Arms the strongest bounded, evidence-backed static path from the selected node.
    ///
    /// This is a presentation of extracted relationships, not an observed runtime trace.
    pub fn arm_selected_trace(&mut self) -> bool {
        let Some(selected_id) = self
            .selected_node
            .and_then(|index| self.graph.nodes.get(index))
            .map(|node| node.id.clone())
        else {
            return false;
        };
        let Some(trace) = path_explorer::selected_static_path(&self.graph, &selected_id) else {
            return false;
        };
        self.active_trace = Some(trace);
        if !self.select_static_flow_hop(0) {
            self.active_trace = None;
            return false;
        }
        true
    }

    fn current_flow(&self) -> Option<&model::Flow> {
        self.active_trace.as_ref()
    }

    fn is_fixture(&self) -> bool {
        self.graph.scan_summary.source == "synthetic_fixture"
    }

    fn clear_trace_context(&mut self) {
        self.flow_hop = None;
        self.relationship_cursor = None;
        if !self.is_fixture() {
            self.active_trace = None;
        }
        self.pause_playback();
        self.animation_frame = 0;
        self.completed = false;
    }

    fn handle_event(&mut self, event: Event) -> bool {
        let Event::Key(key) = event else {
            return matches!(event, Event::Resize(_, _));
        };
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return true;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('t') => self.theme = self.theme.next(),
            KeyCode::Tab => self.active_panel = self.active_panel.next(),
            KeyCode::Esc => self.escape(),
            KeyCode::Char('e') | KeyCode::Enter => self.toggle_inspector(),
            KeyCode::Char('f') => self.toggle_flow(),
            KeyCode::Char(' ') => self.toggle_playback(),
            KeyCode::Char('m') => self.cycle_motion(),
            KeyCode::Char('.') => {
                if self.current_flow().is_some() {
                    self.step_flow(1);
                } else {
                    self.step_relationship(1);
                }
            }
            KeyCode::Char(',') => {
                if self.current_flow().is_some() {
                    self.step_flow(-1);
                } else {
                    self.step_relationship(-1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(0.0, 1.0),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(0.0, -1.0),
            KeyCode::Left | KeyCode::Char('h') => self.move_selection(-1.0, 0.0),
            KeyCode::Right | KeyCode::Char('l') => self.move_selection(1.0, 0.0),
            _ => return false,
        }
        true
    }

    fn toggle_inspector(&mut self) {
        self.active_panel = if self.active_panel == Panel::Inspector {
            Panel::Map
        } else {
            Panel::Inspector
        };
    }

    fn toggle_flow(&mut self) {
        if self.flow_hop.is_some() || (!self.is_fixture() && self.active_trace.is_some()) {
            self.clear_trace_context();
            self.active_panel = Panel::Map;
            if !self.is_fixture() {
                self.map_mode = MapMode::Focus;
            }
        } else {
            let _ = if self.is_fixture() {
                self.select_static_flow_hop(0)
            } else {
                self.arm_selected_trace()
            };
        }
    }

    fn escape(&mut self) {
        if self.active_panel != Panel::Map {
            self.active_panel = Panel::Map;
        } else if self.is_fixture() {
            self.clear_trace_context();
        } else {
            match self.map_mode {
                MapMode::Trace => {
                    self.clear_trace_context();
                    self.map_mode = MapMode::Focus;
                }
                MapMode::Focus => {
                    self.clear_trace_context();
                    self.map_mode = MapMode::Overview;
                }
                MapMode::Overview => {}
            }
        }
    }

    fn toggle_playback(&mut self) {
        if self.playback == PlaybackState::Playing {
            self.pause_playback();
        } else {
            let _ = self.start_static_playback();
        }
    }

    fn cycle_motion(&mut self) {
        self.set_motion_mode(self.motion.next());
    }

    fn move_selection(&mut self, dx: f64, dy: f64) {
        if !self.is_fixture() {
            self.clear_trace_context();
            self.map_mode = MapMode::Focus;
        }
        let Some(current) = self
            .selected_node
            .and_then(|index| self.graph.nodes.get(index))
        else {
            self.selected_node = (!self.graph.nodes.is_empty()).then_some(0);
            if self.is_fixture() {
                self.clear_trace_context();
            }
            return;
        };
        let Some(current_position) = self.layout.position(&current.id) else {
            return;
        };
        let next = self
            .graph
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let position = self.layout.position(&node.id)?;
                let delta_x = position.x - current_position.x;
                let delta_y = position.y - current_position.y;
                let forward = delta_x * dx + delta_y * dy;
                if forward <= 0.0 {
                    return None;
                }
                let sideways = (delta_x * dy - delta_y * dx).abs();
                Some((index, forward + sideways * 2.5))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(index, _)| index);

        if let Some(index) = next {
            self.selected_node = Some(index);
            self.relationship_cursor = None;
            if self.is_fixture() {
                self.clear_trace_context();
            }
        }
    }

    fn outgoing_edge_indices(&self) -> Vec<usize> {
        let Some(selected) = self
            .selected_node
            .and_then(|index| self.graph.nodes.get(index))
        else {
            return Vec::new();
        };
        let mut indices: Vec<_> = self
            .graph
            .edges
            .iter()
            .enumerate()
            .filter_map(|(index, edge)| (edge.source == selected.id).then_some(index))
            .collect();
        indices.sort_by(|left, right| {
            let left = &self.graph.edges[*left];
            let right = &self.graph.edges[*right];
            left.evidence
                .path
                .cmp(&right.evidence.path)
                .then_with(|| left.evidence.line_start.cmp(&right.evidence.line_start))
                .then_with(|| left.evidence.line_end.cmp(&right.evidence.line_end))
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| left.id.cmp(&right.id))
        });
        indices
    }

    fn selected_relationship_edge(
        &self,
    ) -> Option<(usize, usize, &model::Edge, Option<&model::Node>)> {
        let cursor = self.relationship_cursor?;
        let indices = self.outgoing_edge_indices();
        let edge = self.graph.edges.get(*indices.get(cursor)?)?;
        let target = self.graph.nodes.iter().find(|node| node.id == edge.target);
        Some((cursor, indices.len(), edge, target))
    }

    fn step_relationship(&mut self, direction: isize) {
        let total = self.outgoing_edge_indices().len();
        if total == 0 {
            self.relationship_cursor = None;
            return;
        }
        let last = total - 1;
        let next = match (self.relationship_cursor, direction.is_positive()) {
            (None, true) => 0,
            (None, false) => last,
            (Some(index), true) => (index + 1).min(last),
            (Some(index), false) => index.saturating_sub(1),
        };
        self.relationship_cursor = Some(next);
        self.active_panel = Panel::Inspector;
        if !self.is_fixture() {
            self.map_mode = MapMode::Focus;
        }
    }

    fn step_flow(&mut self, direction: isize) {
        let Some(flow) = self.current_flow() else {
            self.flow_hop = None;
            self.pause_playback();
            self.completed = false;
            return;
        };
        if flow.edge_ids.is_empty() || flow.node_ids.len() < 2 {
            self.flow_hop = None;
            self.pause_playback();
            self.completed = false;
            return;
        }
        let last = flow.edge_ids.len().saturating_sub(1);
        let next = match (self.flow_hop, direction.is_positive()) {
            (None, true) => 0,
            (None, false) => last,
            (Some(hop), true) => (hop + 1).min(last),
            (Some(hop), false) => hop.saturating_sub(1),
        };
        let _ = self.select_static_flow_hop(next);
    }

    fn time_until_tick(&self, now: Instant) -> Option<Duration> {
        if self.playback != PlaybackState::Playing {
            return None;
        }
        self.next_tick
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    fn schedule_next_tick(&mut self, now: Instant) {
        let interval = match self.motion {
            MotionMode::Full => FULL_MOTION_INTERVAL,
            MotionMode::Reduced => DISCRETE_MOTION_INTERVAL,
            MotionMode::Off => {
                self.next_tick = None;
                return;
            }
        };
        self.next_tick = Some(now + interval);
    }

    fn pause_playback(&mut self) {
        self.playback = PlaybackState::Paused;
        self.next_tick = None;
    }

    fn advance_playback(&mut self, now: Instant) -> bool {
        if self.playback != PlaybackState::Playing || self.motion == MotionMode::Off {
            return false;
        }
        if self.motion == MotionMode::Full && self.animation_frame < FRAMES_PER_HOP {
            self.animation_frame += 1;
            if self.animation_frame < FRAMES_PER_HOP {
                self.schedule_next_tick(now);
                return true;
            }
        }
        let Some(hop) = self.flow_hop else {
            self.pause_playback();
            return true;
        };
        let Some(last) = self
            .current_flow()
            .and_then(|flow| flow.edge_ids.len().checked_sub(1))
        else {
            self.pause_playback();
            return true;
        };
        if hop >= last {
            self.pause_playback();
            self.animation_frame = FRAMES_PER_HOP;
            self.completed = true;
            return true;
        }
        let active_panel = self.active_panel;
        if self.select_static_flow_hop(hop + 1) {
            self.active_panel = active_panel;
            self.playback = PlaybackState::Playing;
            self.schedule_next_tick(now);
        } else {
            self.pause_playback();
        }
        true
    }

    fn playback_progress(&self) -> f64 {
        if self.completed {
            return 1.0;
        }
        if self.motion != MotionMode::Full {
            return 1.0;
        }
        f64::from(self.animation_frame) / f64::from(FRAMES_PER_HOP)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    fn scanned_graph(node_ids: &[&str], connections: &[(&str, &str)]) -> model::Graph {
        model::Graph {
            schema_version: 1,
            repository: "PROJECT".into(),
            nodes: node_ids
                .iter()
                .map(|id| model::Node {
                    id: (*id).into(),
                    group: "SRC".into(),
                    label: (*id).into(),
                    kind: model::NodeKind::Module,
                    evidence: model::Evidence {
                        path: format!("src/{id}.rs"),
                        line_start: 1,
                        line_end: 1,
                    },
                })
                .collect(),
            edges: connections
                .iter()
                .enumerate()
                .map(|(index, (source, target))| model::Edge {
                    id: format!("E{index}"),
                    source: (*source).into(),
                    target: (*target).into(),
                    relationship: "imports".into(),
                    provenance: model::Provenance::Extracted,
                    confidence: 1.0,
                    evidence: model::Evidence {
                        path: format!("src/{source}.rs"),
                        line_start: 1,
                        line_end: 1,
                    },
                    import_specifier: Some((*target).into()),
                })
                .collect(),
            flows: Vec::new(),
            scan_summary: model::ScanSummary {
                source: "local_static_scan".into(),
                files_discovered: node_ids.len() as u32,
                files_scanned: node_ids.len() as u32,
                files_skipped: 0,
                skipped_by_reason: Default::default(),
                parse_warnings: 0,
                traversal_errors: 0,
                inferred_edges: 0,
            },
        }
    }

    #[test]
    fn theme_cycles_without_changing_graph_state() {
        let mut app = App::new().expect("fixture should load");
        let nodes = app.graph.nodes.len();

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::NONE,
        )));

        assert_eq!(app.theme, ThemeName::GreenRadar);
        assert_eq!(app.graph.nodes.len(), nodes);
        assert!(!app.should_quit);
    }

    #[test]
    fn q_requests_a_clean_exit() {
        let mut app = App::new().expect("fixture should load");
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )));
        assert!(app.should_quit);
    }

    #[test]
    fn control_c_requests_a_clean_exit_from_raw_mode() {
        let mut app = App::new().expect("fixture should load");

        assert!(app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))));

        assert!(app.should_quit);
    }

    #[test]
    fn scanned_map_transitions_from_overview_to_focus_to_trace_and_back() {
        let graph = scanned_graph(
            &["ROOT", "MID", "LEAF"],
            &[("ROOT", "MID"), ("MID", "LEAF")],
        );
        let mut app = App::from_graph(graph);

        assert_eq!(app.map_mode, MapMode::Overview);
        assert!(app.current_flow().is_none());
        assert!(!app.start_static_playback());

        app.move_selection(1.0, 0.0);
        assert_eq!(app.map_mode, MapMode::Focus);
        assert!(app.current_flow().is_none());

        app.selected_node = Some(0);
        assert!(app.arm_selected_trace());
        assert_eq!(app.map_mode, MapMode::Trace);
        assert!(app.current_flow().is_some());
        assert!(app.start_static_playback());
        app.pause_playback();

        app.toggle_flow();
        assert_eq!(app.map_mode, MapMode::Focus);
        assert!(app.current_flow().is_none());
        assert_eq!(app.flow_hop, None);

        assert!(app.arm_selected_trace());
        app.active_panel = Panel::Map;
        app.escape();
        assert_eq!(app.map_mode, MapMode::Focus);
        assert!(app.current_flow().is_none());
        app.escape();
        assert_eq!(app.map_mode, MapMode::Overview);
    }

    #[test]
    fn scanned_graph_initially_selects_the_first_root_candidate() {
        let app = App::from_graph(scanned_graph(
            &["CHILD", "ROOT", "LEAF"],
            &[("ROOT", "CHILD"), ("CHILD", "LEAF")],
        ));

        assert_eq!(app.selected_node, Some(1));
        assert_eq!(app.graph.nodes[app.selected_node.unwrap()].id, "ROOT");
    }

    #[test]
    fn explicit_entry_selection_precedes_a_scanned_root_candidate() {
        let mut graph = scanned_graph(
            &["CHILD", "ROOT", "ENTRY"],
            &[("ROOT", "CHILD"), ("CHILD", "ENTRY")],
        );
        graph.nodes[2].kind = model::NodeKind::Entry;

        let app = App::from_graph(graph);

        assert_eq!(app.selected_node, Some(2));
        assert_eq!(app.graph.nodes[app.selected_node.unwrap()].id, "ENTRY");
    }

    #[test]
    fn cyclic_scanned_graph_falls_back_to_the_first_node() {
        let app = App::from_graph(scanned_graph(&["A", "B"], &[("A", "B"), ("B", "A")]));

        assert_eq!(app.selected_node, Some(0));
        assert_eq!(app.graph.nodes[app.selected_node.unwrap()].id, "A");
        assert_eq!(app.map_mode, MapMode::Overview);
    }

    #[test]
    fn synthetic_fixture_keeps_its_trace_first_state_contract() {
        let mut app = App::new().expect("fixture should load");

        assert_eq!(app.map_mode, MapMode::Trace);
        assert_eq!(app.selected_node, Some(0));
        assert!(app.current_flow().is_some());
        assert_eq!(app.flow_hop, None);

        app.toggle_flow();
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.active_panel, Panel::Inspector);
        app.escape();
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.active_panel, Panel::Map);
        app.escape();
        assert_eq!(app.flow_hop, None);
        assert_eq!(app.map_mode, MapMode::Trace);
    }

    #[test]
    fn empty_and_no_entry_graphs_have_explicit_deterministic_selection() {
        let mut empty = load_beacon_ops().expect("fixture should load");
        empty.repository = "EMPTY".into();
        empty.scan_summary.source = "local_static_scan".into();
        empty.nodes.clear();
        empty.edges.clear();
        empty.flows.clear();
        let mut empty_app = App::from_graph(empty);
        assert_eq!(empty_app.selected_node, None);
        empty_app.move_selection(1.0, 0.0);
        assert_eq!(empty_app.selected_node, None);

        let mut no_entry = load_beacon_ops().expect("fixture should load");
        for node in &mut no_entry.nodes {
            node.kind = model::NodeKind::Module;
        }
        let app = App::from_graph(no_entry);
        assert_eq!(app.selected_node, Some(0));
    }

    #[test]
    fn tab_and_escape_cycle_focus_without_changing_selection() {
        let mut app = App::new().expect("fixture should load");

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(app.active_panel, Panel::Navigator);
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(app.active_panel, Panel::Inspector);
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(app.active_panel, Panel::Map);
        assert_eq!(app.selected_node, Some(0));
    }

    #[test]
    fn flow_steps_keep_node_and_edge_evidence_synchronized() {
        let mut app = App::new().expect("fixture should load");

        app.step_flow(1);
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.graph.nodes[app.selected_node.unwrap()].id, "N02");
        assert_eq!(app.active_panel, Panel::Inspector);

        app.step_flow(1);
        assert_eq!(app.flow_hop, Some(1));
        assert_eq!(app.graph.nodes[app.selected_node.unwrap()].id, "N09");

        app.step_flow(-1);
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.graph.nodes[app.selected_node.unwrap()].id, "N02");
    }

    #[test]
    fn directional_navigation_clears_flow_context() {
        let mut app = App::new().expect("fixture should load");
        app.step_flow(1);

        app.move_selection(1.0, 0.0);

        assert_eq!(app.flow_hop, None);
        assert_ne!(app.graph.nodes[app.selected_node.unwrap()].id, "N02");
    }

    #[test]
    fn flow_toggle_and_escape_preserve_a_literal_path_context() {
        let mut app = App::new().expect("fixture should load");

        app.toggle_flow();
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.active_panel, Panel::Inspector);

        app.escape();
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.active_panel, Panel::Map);

        app.escape();
        assert_eq!(app.flow_hop, None);
    }

    #[test]
    fn flow_selection_rejects_a_stale_contract_without_mutating_state() {
        let mut app = App::new().expect("fixture should load");
        app.active_trace.as_mut().unwrap().edge_ids[0] = "MISSING".into();

        assert!(!app.select_static_flow_hop(0));
        assert_eq!(app.flow_hop, None);
        assert_eq!(app.selected_node, Some(0));
        assert_eq!(app.active_panel, Panel::Map);
    }

    #[test]
    fn scanned_graph_traces_outward_from_selection_without_mutating_contract() {
        let mut graph = load_beacon_ops().expect("fixture should load");
        graph.repository = "PROJECT".into();
        graph.scan_summary.source = "local_static_scan".into();
        graph.flows.clear();
        let serialized_before = serde_json::to_string(&graph).unwrap();
        let mut app = App::from_graph(graph);

        assert!(app.current_flow().is_none());
        assert!(app.arm_selected_trace());
        let trace = app.current_flow().expect("selected trace should be armed");
        assert_eq!(trace.node_ids.first().map(String::as_str), Some("N01"));
        assert!(trace.edge_ids.len() <= 12);
        assert!(trace.edge_ids.iter().all(|edge_id| {
            app.graph
                .edges
                .iter()
                .any(|edge| edge.id == *edge_id && edge.provenance == model::Provenance::Extracted)
        }));
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(
            serde_json::to_string(&app.graph).unwrap(),
            serialized_before,
            "an interactive trace must not mutate exported scanner data"
        );

        app.move_selection(1.0, 0.0);
        assert!(app.current_flow().is_none());
        assert_eq!(app.flow_hop, None);
    }

    #[test]
    fn every_outgoing_relationship_is_reachable_in_deterministic_evidence_order() {
        let mut graph = load_beacon_ops().expect("fixture should load");
        graph.scan_summary.source = "local_static_scan".into();
        graph.flows.clear();
        let mut app = App::from_graph(graph);
        app.selected_node = app.graph.nodes.iter().position(|node| node.id == "N02");

        app.step_relationship(1);
        let first = app
            .selected_relationship_edge()
            .expect("first relationship should be selected");
        assert_eq!((first.0, first.1), (0, 5));
        let first_path = first.2.evidence.path.clone();
        assert_eq!(app.active_panel, Panel::Inspector);
        assert_eq!(app.map_mode, MapMode::Focus);

        for _ in 0..8 {
            app.step_relationship(1);
        }
        let last = app
            .selected_relationship_edge()
            .expect("last relationship should remain selected");
        assert_eq!((last.0, last.1), (4, 5));
        assert!(first_path <= last.2.evidence.path);

        app.move_selection(1.0, 0.0);
        assert_eq!(app.relationship_cursor, None);
    }

    #[test]
    fn leaf_selection_cannot_claim_an_outward_trace() {
        let mut graph = load_beacon_ops().expect("fixture should load");
        graph.scan_summary.source = "local_static_scan".into();
        graph.flows.clear();
        let mut app = App::from_graph(graph);
        app.selected_node = app.graph.nodes.iter().position(|node| node.id == "N23");

        assert!(!app.arm_selected_trace());
        assert!(app.current_flow().is_none());
        assert_eq!(app.flow_hop, None);
    }

    #[test]
    fn rendered_command_bindings_drive_the_expected_state() {
        let mut app = App::new().expect("fixture should load");
        let press = |app: &mut App, code| {
            app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        };

        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.flow_hop, Some(0));
        press(&mut app, KeyCode::Char('.'));
        assert_eq!(app.flow_hop, Some(1));
        press(&mut app, KeyCode::Char(','));
        assert_eq!(app.flow_hop, Some(0));
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.active_panel, Panel::Map);
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.active_panel, Panel::Inspector);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.flow_hop, None);
    }

    #[test]
    fn playback_blocks_when_idle_and_caps_full_motion_at_twelve_fps() {
        let mut app = App::new().expect("fixture should load");
        assert_eq!(app.time_until_tick(Instant::now()), None);

        app.toggle_playback();
        assert_eq!(app.playback, PlaybackState::Playing);
        assert_eq!(app.active_panel, Panel::Map);
        assert!(
            app.time_until_tick(Instant::now())
                .is_some_and(|wait| wait <= Duration::from_millis(84))
        );

        for _ in 0..11 {
            app.advance_static_playback();
        }
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.animation_frame, 11);

        app.advance_static_playback();
        assert_eq!(app.flow_hop, Some(1));
        assert_eq!(app.animation_frame, 0);
        assert_eq!(app.playback, PlaybackState::Playing);
        assert_eq!(app.active_panel, Panel::Map);

        app.toggle_playback();
        assert_eq!(app.playback, PlaybackState::Paused);
        assert_eq!(app.time_until_tick(Instant::now()), None);
    }

    #[test]
    fn reduced_motion_is_discrete_and_off_motion_is_manual_only() {
        let mut app = App::new().expect("fixture should load");
        app.toggle_playback();
        app.cycle_motion();
        assert_eq!(app.motion, MotionMode::Reduced);
        assert_eq!(app.time_until_tick(Instant::now()), None);
        app.toggle_playback();
        assert!(
            app.time_until_tick(Instant::now())
                .is_some_and(|wait| wait <= Duration::from_secs(1))
        );
        app.advance_static_playback();
        assert_eq!(app.flow_hop, Some(1));

        app.cycle_motion();
        assert_eq!(app.motion, MotionMode::Off);
        assert_eq!(app.time_until_tick(Instant::now()), None);
        app.toggle_playback();
        assert_eq!(app.playback, PlaybackState::Paused);
        assert_eq!(app.time_until_tick(Instant::now()), None);
        app.step_flow(1);
        assert_eq!(app.flow_hop, Some(2));
    }

    #[test]
    fn playback_stops_on_the_final_literal_edge() {
        let mut app = App::new().expect("fixture should load");
        let last = app.graph.flows[0].edge_ids.len() - 1;
        assert!(app.select_static_flow_hop(last));
        app.toggle_playback();

        for _ in 0..FRAMES_PER_HOP {
            app.advance_static_playback();
        }

        assert_eq!(app.flow_hop, Some(last));
        assert_eq!(app.playback, PlaybackState::Paused);
        assert_eq!(app.animation_frame, FRAMES_PER_HOP);
        assert!(app.completed);
        assert_eq!(app.time_until_tick(Instant::now()), None);

        app.cycle_motion();
        app.cycle_motion();
        app.cycle_motion();
        assert_eq!(app.motion, MotionMode::Full);
        assert_eq!(app.playback_progress(), 1.0);
        assert!(app.completed);
        app.toggle_playback();
        assert_eq!(app.flow_hop, Some(0));
        assert_eq!(app.animation_frame, 0);
        assert_eq!(app.playback, PlaybackState::Playing);
        assert!(!app.completed);
    }

    #[test]
    fn only_real_input_or_resize_requests_a_redraw() {
        let mut app = App::new().expect("fixture should load");

        assert!(!app.handle_event(Event::FocusGained));
        assert!(app.handle_event(Event::Resize(120, 34)));
        assert!(!app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        ))));

        let mut release = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(!app.handle_event(Event::Key(release)));

        assert!(app.start_static_playback());
        let deadline = app.next_tick;
        assert!(app.handle_event(Event::Resize(140, 40)));
        assert_eq!(app.next_tick, deadline, "events must not postpone playback");
        assert_eq!(
            app.time_until_tick(deadline.expect("playback should own a deadline")),
            Some(Duration::ZERO),
            "an overdue tick must be recognized before polling for more events"
        );
    }

    #[test]
    fn the_fixture_flow_finishes_after_exactly_108_full_motion_ticks() {
        let mut app = App::new().expect("fixture should load");
        assert!(app.start_static_playback());

        for _ in 0..108 {
            assert!(app.advance_static_playback());
        }

        assert_eq!(app.flow_hop, Some(8));
        assert_eq!(app.animation_frame, FRAMES_PER_HOP);
        assert_eq!(app.playback, PlaybackState::Paused);
        assert!(app.completed);
        assert_eq!(app.time_until_tick(Instant::now()), None);
    }
}

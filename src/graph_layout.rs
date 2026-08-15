use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ratatui::layout::Rect;

use crate::model::{Graph, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupBounds {
    pub label: &'static str,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutGroup {
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphLayout {
    positions: BTreeMap<String, Point>,
    groups: Vec<LayoutGroup>,
    cycle_edges: BTreeSet<String>,
}

impl GraphLayout {
    pub fn for_graph(graph: &Graph) -> Self {
        let cycle_edges = find_cycle_edges(graph);
        if graph.scan_summary.source == "synthetic_fixture"
            && graph
                .nodes
                .iter()
                .all(|node| node_position(&node.id).is_some())
        {
            return Self {
                positions: graph
                    .nodes
                    .iter()
                    .filter_map(|node| {
                        node_position(&node.id).map(|point| (node.id.clone(), point))
                    })
                    .collect(),
                groups: GROUP_BOUNDS
                    .iter()
                    .map(|group| LayoutGroup {
                        label: group.label.to_owned(),
                        x: group.x,
                        y: group.y,
                        width: group.width,
                        height: group.height,
                    })
                    .collect(),
                cycle_edges,
            };
        }

        generic_layout(graph, cycle_edges)
    }

    pub fn position(&self, id: &str) -> Option<Point> {
        self.positions.get(id).copied()
    }

    pub fn groups(&self) -> &[LayoutGroup] {
        &self.groups
    }

    pub fn is_cycle_edge(&self, id: &str) -> bool {
        self.cycle_edges.contains(id)
    }

    pub fn cycle_edges(&self) -> &BTreeSet<String> {
        &self.cycle_edges
    }
}

pub const GROUP_BOUNDS: [GroupBounds; 7] = [
    GroupBounds {
        label: "UI // CLIENT",
        x: 2.0,
        y: 47.0,
        width: 43.0,
        height: 11.0,
    },
    GroupBounds {
        label: "BOOT",
        x: 5.0,
        y: 24.0,
        width: 18.0,
        height: 22.0,
    },
    GroupBounds {
        label: "HTTP",
        x: 24.0,
        y: 26.0,
        width: 20.0,
        height: 20.0,
    },
    GroupBounds {
        label: "DOMAIN",
        x: 45.0,
        y: 16.0,
        width: 20.0,
        height: 36.0,
    },
    GroupBounds {
        label: "DATA",
        x: 66.0,
        y: 15.0,
        width: 27.0,
        height: 26.0,
    },
    GroupBounds {
        label: "EXT // BOUNDARY",
        x: 47.0,
        y: 2.0,
        width: 25.0,
        height: 13.0,
    },
    GroupBounds {
        label: "OPS",
        x: 27.0,
        y: 1.0,
        width: 19.0,
        height: 10.0,
    },
];

pub fn node_position(id: &str) -> Option<Point> {
    let (x, y) = match id {
        "N01" => (7.0, 39.0),
        "N02" => (17.0, 33.0),
        "N03" => (13.0, 28.0),
        "N04" => (6.0, 54.0),
        "N05" => (16.0, 54.0),
        "N06" => (26.0, 54.0),
        "N07" => (40.0, 55.0),
        "N08" => (34.0, 49.0),
        "N09" => (26.0, 38.0),
        "N10" => (33.0, 31.0),
        "N11" => (41.0, 36.0),
        "N12" => (31.0, 43.0),
        "N13" => (40.0, 43.0),
        "N14" => (49.0, 30.0),
        "N15" => (58.0, 36.0),
        "N16" => (51.0, 46.0),
        "N17" => (60.0, 46.0),
        "N18" => (56.0, 20.0),
        "N19" => (67.0, 28.0),
        "N20" => (77.0, 34.0),
        "N21" => (71.0, 18.0),
        "N22" => (76.0, 37.0),
        "N23" => (88.0, 28.0),
        "N24" => (52.0, 9.0),
        "N25" => (65.0, 6.0),
        "N26" => (67.0, 12.0),
        "N27" => (68.0, 20.0),
        "N28" => (34.0, 7.0),
        "N29" => (43.0, 4.0),
        _ => return None,
    };
    Some(Point { x, y })
}

fn generic_layout(graph: &Graph, cycle_edges: BTreeSet<String>) -> GraphLayout {
    const LEFT: f64 = 2.0;
    const BOTTOM: f64 = 2.0;
    const WIDTH: f64 = 96.0;
    const HEIGHT: f64 = 56.0;
    const GAP: f64 = 2.0;

    let depths = node_depths(graph);
    let mut grouped_nodes: BTreeMap<String, Vec<&crate::model::Node>> = BTreeMap::new();
    for node in &graph.nodes {
        grouped_nodes
            .entry(node.group.clone())
            .or_default()
            .push(node);
    }

    for nodes in grouped_nodes.values_mut() {
        nodes.sort_by(|left, right| {
            node_is_external(left)
                .cmp(&node_is_external(right))
                .then_with(|| {
                    depths
                        .get(&left.id)
                        .copied()
                        .unwrap_or(usize::MAX)
                        .cmp(&depths.get(&right.id).copied().unwrap_or(usize::MAX))
                })
                .then_with(|| left.evidence.path.cmp(&right.evidence.path))
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    let mut group_names: Vec<_> = grouped_nodes.keys().cloned().collect();
    group_names.sort_by(|left, right| {
        let left_external = group_is_external(grouped_nodes.get(left).expect("group exists"));
        let right_external = group_is_external(grouped_nodes.get(right).expect("group exists"));
        let left_depth = group_depth(grouped_nodes.get(left).expect("group exists"), &depths);
        let right_depth = group_depth(grouped_nodes.get(right).expect("group exists"), &depths);
        left_external
            .cmp(&right_external)
            .then_with(|| left_depth.cmp(&right_depth))
            .then_with(|| left.cmp(right))
    });

    let group_count = group_names.len();
    if group_count == 0 {
        return GraphLayout {
            positions: BTreeMap::new(),
            groups: Vec::new(),
            cycle_edges,
        };
    }

    let proposed_columns = ((group_count as f64 * (WIDTH / HEIGHT)).sqrt().ceil() as usize)
        .max(1)
        .min(group_count);
    let rows = group_count.div_ceil(proposed_columns);
    let columns = group_count.div_ceil(rows);
    let group_width = ((WIDTH - GAP * columns.saturating_sub(1) as f64) / columns as f64).max(1.0);
    let group_height = ((HEIGHT - GAP * rows.saturating_sub(1) as f64) / rows as f64).max(1.0);

    let mut positions = BTreeMap::new();
    let mut groups = Vec::with_capacity(group_count);
    for (index, group_name) in group_names.into_iter().enumerate() {
        // Column-major packing makes later dependency layers, especially external
        // boundaries, progress toward the right side of the instrument.
        let column = index / rows;
        let row = index % rows;
        let x = LEFT + column as f64 * (group_width + GAP);
        let y = BOTTOM + (rows - row - 1) as f64 * (group_height + GAP);
        let bounds = LayoutGroup {
            label: group_name.clone(),
            x,
            y,
            width: group_width,
            height: group_height,
        };

        if let Some(nodes) = grouped_nodes.get(&group_name) {
            place_group_nodes(nodes, &bounds, &mut positions);
        }
        groups.push(bounds);
    }

    GraphLayout {
        positions,
        groups,
        cycle_edges,
    }
}

fn group_is_external(nodes: &[&crate::model::Node]) -> bool {
    nodes.iter().any(|node| node_is_external(node))
}

fn node_is_external(node: &crate::model::Node) -> bool {
    matches!(
        node.kind,
        NodeKind::ExternalPackage
            | NodeKind::ExternalService
            | NodeKind::ExternalSystem
            | NodeKind::Unresolved
    )
}

fn group_depth(nodes: &[&crate::model::Node], depths: &BTreeMap<String, usize>) -> usize {
    nodes
        .iter()
        .filter_map(|node| depths.get(&node.id).copied())
        .min()
        .unwrap_or(usize::MAX)
}

fn place_group_nodes(
    nodes: &[&crate::model::Node],
    bounds: &LayoutGroup,
    positions: &mut BTreeMap<String, Point>,
) {
    if nodes.is_empty() {
        return;
    }

    const PADDING_X: f64 = 1.5;
    const PADDING_Y: f64 = 1.5;
    let usable_width = (bounds.width - PADDING_X * 2.0).max(0.5);
    let usable_height = (bounds.height - PADDING_Y * 2.0).max(0.5);
    let columns = ((nodes.len() as f64 * (usable_width / usable_height).max(0.25))
        .sqrt()
        .ceil() as usize)
        .max(1)
        .min(nodes.len());
    let rows = nodes.len().div_ceil(columns);

    for (index, node) in nodes.iter().enumerate() {
        let column = index / rows;
        let row = index % rows;
        let x = bounds.x + PADDING_X + (column + 1) as f64 * usable_width / (columns + 1) as f64;
        let y = bounds.y + bounds.height
            - PADDING_Y
            - (row + 1) as f64 * usable_height / (rows + 1) as f64;
        positions.insert(
            node.id.clone(),
            Point {
                x: x.clamp(0.0, 100.0),
                y: y.clamp(0.0, 60.0),
            },
        );
    }
}

fn node_depths(graph: &Graph) -> BTreeMap<String, usize> {
    let node_ids: BTreeSet<_> = graph.nodes.iter().map(|node| node.id.clone()).collect();
    let mut incoming: BTreeMap<String, usize> =
        node_ids.iter().map(|id| (id.clone(), 0_usize)).collect();
    let mut outgoing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in &graph.edges {
        if node_ids.contains(&edge.source) && node_ids.contains(&edge.target) {
            outgoing
                .entry(edge.source.clone())
                .or_default()
                .insert(edge.target.clone());
            *incoming.entry(edge.target.clone()).or_default() += 1;
        }
    }

    let entry_ids: BTreeSet<_> = graph
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Entry))
        .map(|node| node.id.clone())
        .collect();
    let mut seeds: Vec<_> = if entry_ids.is_empty() {
        incoming
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| id.clone())
            .collect()
    } else {
        entry_ids.into_iter().collect()
    };
    if seeds.is_empty() {
        seeds.extend(node_ids.iter().cloned());
    }

    let mut depths: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        depths.insert(seed.clone(), 0);
        queue.push_back(seed);
    }
    while let Some(source) = queue.pop_front() {
        let depth = depths[&source];
        if let Some(targets) = outgoing.get(&source) {
            for target in targets {
                let next_depth = depth.saturating_add(1);
                if depths.get(target).is_none_or(|known| next_depth < *known) {
                    depths.insert(target.clone(), next_depth);
                    queue.push_back(target.clone());
                }
            }
        }
    }
    depths
}

fn find_cycle_edges(graph: &Graph) -> BTreeSet<String> {
    let node_ids: BTreeSet<_> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut adjacency: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in &graph.edges {
        if node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str()) {
            adjacency
                .entry(edge.source.as_str())
                .or_default()
                .insert(edge.target.as_str());
        }
    }

    graph
        .edges
        .iter()
        .filter(|edge| {
            node_ids.contains(edge.source.as_str())
                && node_ids.contains(edge.target.as_str())
                && (edge.source == edge.target
                    || path_exists(edge.target.as_str(), edge.source.as_str(), &adjacency))
        })
        .map(|edge| edge.id.clone())
        .collect()
}

fn path_exists(start: &str, goal: &str, adjacency: &BTreeMap<&str, BTreeSet<&str>>) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if node == goal {
            return true;
        }
        if !visited.insert(node) {
            continue;
        }
        if let Some(targets) = adjacency.get(node) {
            pending.extend(targets.iter().copied());
        }
    }
    false
}

pub fn canvas_to_cell(point: Point, area: Rect) -> (u16, u16) {
    let usable_width = area.width.saturating_sub(1);
    let usable_height = area.height.saturating_sub(1);
    let x = area.x + ((point.x / 100.0) * f64::from(usable_width)).round() as u16;
    let y = area.y + (((60.0 - point.y) / 60.0) * f64::from(usable_height)).round() as u16;
    (
        x.min(area.right().saturating_sub(1)),
        y.min(area.bottom().saturating_sub(1)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        fixture::load_beacon_ops,
        model::{Edge, Evidence, Flow, Graph, Node, Provenance, ScanSummary},
    };

    #[test]
    fn every_fixture_node_has_a_stable_position() {
        for index in 1..=29 {
            assert!(node_position(&format!("N{index:02}")).is_some());
        }
    }

    #[test]
    fn coordinate_mapping_stays_inside_the_canvas() {
        let area = Rect::new(10, 5, 80, 20);
        for index in 1..=29 {
            let point = node_position(&format!("N{index:02}")).expect("position should exist");
            let (x, y) = canvas_to_cell(point, area);
            assert!(x >= area.left() && x < area.right());
            assert!(y >= area.top() && y < area.bottom());
        }
    }

    #[test]
    fn beacon_graph_layout_preserves_reviewed_geometry() {
        let graph = load_beacon_ops().expect("fixture should load");
        let layout = GraphLayout::for_graph(&graph);

        for node in &graph.nodes {
            assert_eq!(layout.position(&node.id), node_position(&node.id));
        }
        assert_eq!(layout.groups().len(), GROUP_BOUNDS.len());
        for (actual, expected) in layout.groups().iter().zip(GROUP_BOUNDS) {
            assert_eq!(actual.label, expected.label);
            assert_eq!(actual.x, expected.x);
            assert_eq!(actual.y, expected.y);
            assert_eq!(actual.width, expected.width);
            assert_eq!(actual.height, expected.height);
        }
        assert!(layout.is_cycle_edge("E23"));
        assert!(layout.is_cycle_edge("E24"));
    }

    #[test]
    fn generic_layout_is_deterministic_under_input_permutation() {
        let graph = generic_graph(24, 6);
        let expected = GraphLayout::for_graph(&graph);
        let mut permuted = graph.clone();
        permuted.nodes.reverse();
        permuted.edges.reverse();

        assert_eq!(GraphLayout::for_graph(&permuted), expected);
    }

    #[test]
    fn forty_groups_stay_bounded_and_do_not_overlap() {
        let graph = generic_graph(40, 40);
        let layout = GraphLayout::for_graph(&graph);

        assert_eq!(layout.groups().len(), 40);
        for node in &graph.nodes {
            let point = layout
                .position(&node.id)
                .expect("node should be positioned");
            assert!((0.0..=100.0).contains(&point.x));
            assert!((0.0..=60.0).contains(&point.y));
        }
        for (index, left) in layout.groups().iter().enumerate() {
            assert!(left.x >= 0.0 && left.y >= 0.0);
            assert!(left.x + left.width <= 100.0);
            assert!(left.y + left.height <= 60.0);
            for right in layout.groups().iter().skip(index + 1) {
                assert!(
                    !rectangles_overlap(left, right),
                    "{left:?} overlaps {right:?}"
                );
            }
        }
    }

    #[test]
    fn external_and_unresolved_groups_are_packed_to_the_right() {
        let mut graph = generic_graph(4, 4);
        graph.nodes[2].kind = NodeKind::ExternalPackage;
        graph.nodes[3].kind = NodeKind::Unresolved;
        let layout = GraphLayout::for_graph(&graph);
        let leftmost_boundary = layout
            .groups()
            .iter()
            .filter(|group| matches!(group.label.as_str(), "G02" | "G03"))
            .map(|group| group.x)
            .fold(f64::INFINITY, f64::min);
        let rightmost_internal = layout
            .groups()
            .iter()
            .filter(|group| matches!(group.label.as_str(), "G00" | "G01"))
            .map(|group| group.x)
            .fold(f64::NEG_INFINITY, f64::max);

        assert!(leftmost_boundary >= rightmost_internal);
    }

    #[test]
    fn cycle_lookup_derives_edges_in_cycles_and_self_loops() {
        let mut graph = generic_graph(4, 1);
        graph.edges = vec![
            edge("E01", "N00", "N01"),
            edge("E02", "N01", "N02"),
            edge("E03", "N02", "N00"),
            edge("E04", "N02", "N03"),
            edge("E05", "N03", "N03"),
        ];
        let layout = GraphLayout::for_graph(&graph);

        assert_eq!(
            layout.cycle_edges(),
            &BTreeSet::from([
                "E01".to_owned(),
                "E02".to_owned(),
                "E03".to_owned(),
                "E05".to_owned(),
            ])
        );
    }

    fn generic_graph(node_count: usize, group_count: usize) -> Graph {
        let nodes = (0..node_count)
            .map(|index| Node {
                id: format!("N{index:02}"),
                group: format!("G{:02}", index % group_count),
                label: format!("MODULE.{index:02}"),
                kind: if index == 0 {
                    NodeKind::Entry
                } else {
                    NodeKind::Module
                },
                evidence: Evidence {
                    path: format!("src/module_{index:02}.rs"),
                    line_start: 1,
                    line_end: 1,
                },
            })
            .collect();
        let edges = (1..node_count)
            .map(|index| {
                edge(
                    &format!("E{index:02}"),
                    &format!("N{:02}", index - 1),
                    &format!("N{index:02}"),
                )
            })
            .collect();
        Graph {
            schema_version: 1,
            repository: "GENERIC".to_owned(),
            nodes,
            edges,
            flows: Vec::<Flow>::new(),
            scan_summary: ScanSummary {
                source: "test".to_owned(),
                files_discovered: node_count as u32,
                files_scanned: node_count as u32,
                files_skipped: 0,
                skipped_by_reason: BTreeMap::new(),
                parse_warnings: 0,
                traversal_errors: 0,
                inferred_edges: 0,
            },
        }
    }

    fn edge(id: &str, source: &str, target: &str) -> Edge {
        Edge {
            id: id.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
            relationship: "imports".to_owned(),
            provenance: Provenance::Extracted,
            confidence: 1.0,
            evidence: Evidence {
                path: "src/lib.rs".to_owned(),
                line_start: 1,
                line_end: 1,
            },
            import_specifier: None,
        }
    }

    fn rectangles_overlap(left: &LayoutGroup, right: &LayoutGroup) -> bool {
        left.x < right.x + right.width
            && left.x + left.width > right.x
            && left.y < right.y + right.height
            && left.y + left.height > right.y
    }
}

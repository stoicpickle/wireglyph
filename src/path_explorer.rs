use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{Edge, Flow, Graph, NodeKind, Provenance};

const MAX_HOPS: usize = 12;
const MAX_EDGE_EXPLORATIONS: usize = 20_000;

/// Builds the strongest evidence-backed static path starting at `selected_node_id`.
///
/// "Strongest" means the longest extracted path encountered within twelve hops
/// and a strict exploration budget. Paths with equal length are ordered by each
/// edge's evidence path, line, and ID so the result is stable even when the
/// graph's vectors arrive in a different order. A cycle-closing edge may end
/// the path, but traversal never continues through a repeated node.
pub fn selected_static_path(graph: &Graph, selected_node_id: &str) -> Option<Flow> {
    selected_static_path_with_summary(graph, selected_node_id).flow
}

/// Builds a selected path and reports the deterministic search bounds applied.
pub fn selected_static_path_with_summary(
    graph: &Graph,
    selected_node_id: &str,
) -> StaticPathSelection {
    selected_static_path_bounded(graph, selected_node_id, MAX_EDGE_EXPLORATIONS)
}

#[derive(Clone, Debug)]
pub struct StaticPathSelection {
    pub flow: Option<Flow>,
    pub search: PathSearchSummary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathSearchSummary {
    pub max_hops: u32,
    pub max_edge_explorations: u32,
    pub edge_explorations: u32,
    pub hop_limit_reached: bool,
    pub edge_exploration_limit_reached: bool,
}

fn selected_static_path_bounded(
    graph: &Graph,
    selected_node_id: &str,
    max_edge_explorations: usize,
) -> StaticPathSelection {
    let nodes: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let Some(selected) = nodes.get(selected_node_id) else {
        return empty_selection(max_edge_explorations);
    };
    if is_terminal_kind(&selected.kind) {
        return empty_selection(max_edge_explorations);
    }

    let mut outgoing: BTreeMap<&str, Vec<&Edge>> = BTreeMap::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.provenance == Provenance::Extracted)
        .filter(|edge| nodes.contains_key(edge.source.as_str()))
        .filter(|edge| nodes.contains_key(edge.target.as_str()))
    {
        outgoing.entry(edge.source.as_str()).or_default().push(edge);
    }
    for edges in outgoing.values_mut() {
        edges.sort_by(|left, right| edge_order(left, right));
    }

    let selected_id = selected.id.as_str();
    let mut visited = BTreeSet::from([selected_id]);
    let mut node_ids = vec![selected.id.clone()];
    let mut edge_ids = Vec::new();
    let mut edge_keys = Vec::new();
    let mut best = None;
    let mut budget = SearchBudget::new(max_edge_explorations);
    let mut hop_limit_reached = false;
    search(
        selected_id,
        &nodes,
        &outgoing,
        &mut visited,
        &mut node_ids,
        &mut edge_ids,
        &mut edge_keys,
        &mut best,
        &mut budget,
        &mut hop_limit_reached,
    );

    let Some(best) = best.filter(|path: &Candidate| !path.edge_ids.is_empty()) else {
        return StaticPathSelection {
            flow: None,
            search: budget.summary(hop_limit_reached),
        };
    };
    let id_key = format!(
        "selected-path\0{}\0{}",
        selected_node_id,
        best.edge_ids.join("\0")
    );
    let flow = Flow {
        id: stable_id("P", &id_key),
        label: format!("STATIC PATH FROM {} [{}]", selected.label, selected_node_id),
        provenance: Provenance::Extracted,
        node_ids: best.node_ids,
        edge_ids: best.edge_ids,
    };
    StaticPathSelection {
        flow: Some(flow),
        search: budget.summary(hop_limit_reached),
    }
}

fn empty_selection(exploration_limit: usize) -> StaticPathSelection {
    StaticPathSelection {
        flow: None,
        search: PathSearchSummary {
            max_hops: u32::try_from(MAX_HOPS).unwrap_or(u32::MAX),
            max_edge_explorations: u32::try_from(exploration_limit).unwrap_or(u32::MAX),
            edge_explorations: 0,
            hop_limit_reached: false,
            edge_exploration_limit_reached: false,
        },
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    node_ids: Vec<String>,
    edge_ids: Vec<String>,
    edge_keys: Vec<EdgeKey>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EdgeKey {
    path: String,
    line_start: u32,
    id: String,
}

#[derive(Clone, Copy, Debug)]
struct SearchBudget {
    limit: usize,
    remaining: usize,
    explored: usize,
    limit_reached: bool,
}

impl SearchBudget {
    const fn new(limit: usize) -> Self {
        Self {
            limit,
            remaining: limit,
            explored: 0,
            limit_reached: false,
        }
    }

    fn spend(&mut self) -> bool {
        if self.remaining == 0 {
            self.limit_reached = true;
            return false;
        }
        self.remaining -= 1;
        self.explored += 1;
        true
    }

    fn summary(self, hop_limit_reached: bool) -> PathSearchSummary {
        PathSearchSummary {
            max_hops: u32::try_from(MAX_HOPS).unwrap_or(u32::MAX),
            max_edge_explorations: u32::try_from(self.limit).unwrap_or(u32::MAX),
            edge_explorations: u32::try_from(self.explored).unwrap_or(u32::MAX),
            hop_limit_reached,
            edge_exploration_limit_reached: self.limit_reached,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn search<'a>(
    current: &'a str,
    nodes: &BTreeMap<&'a str, &'a crate::model::Node>,
    outgoing: &BTreeMap<&'a str, Vec<&'a Edge>>,
    visited: &mut BTreeSet<&'a str>,
    node_ids: &mut Vec<String>,
    edge_ids: &mut Vec<String>,
    edge_keys: &mut Vec<EdgeKey>,
    best: &mut Option<Candidate>,
    budget: &mut SearchBudget,
    hop_limit_reached: &mut bool,
) {
    let terminal = nodes
        .get(current)
        .is_none_or(|node| is_terminal_kind(&node.kind));
    let available = outgoing.get(current).map(Vec::as_slice).unwrap_or_default();

    if terminal || available.is_empty() || edge_ids.len() == MAX_HOPS {
        *hop_limit_reached |= edge_ids.len() == MAX_HOPS && !terminal && !available.is_empty();
        consider(node_ids, edge_ids, edge_keys, best);
        return;
    }

    for edge in available {
        if !budget.spend() {
            consider(node_ids, edge_ids, edge_keys, best);
            return;
        }
        edge_ids.push(edge.id.clone());
        edge_keys.push(EdgeKey::from(*edge));
        node_ids.push(edge.target.clone());

        if visited.contains(edge.target.as_str()) {
            consider(node_ids, edge_ids, edge_keys, best);
        } else {
            visited.insert(edge.target.as_str());
            search(
                edge.target.as_str(),
                nodes,
                outgoing,
                visited,
                node_ids,
                edge_ids,
                edge_keys,
                best,
                budget,
                hop_limit_reached,
            );
            visited.remove(edge.target.as_str());
        }

        node_ids.pop();
        edge_keys.pop();
        edge_ids.pop();

        // Twelve hops is the absolute maximum, and sorted depth-first traversal
        // encounters the stable tie winner first.
        if best
            .as_ref()
            .is_some_and(|candidate| candidate.edge_ids.len() == MAX_HOPS)
        {
            return;
        }
    }
}

fn consider(
    node_ids: &[String],
    edge_ids: &[String],
    edge_keys: &[EdgeKey],
    best: &mut Option<Candidate>,
) {
    let replace = best.as_ref().is_none_or(|current| {
        edge_ids.len() > current.edge_ids.len()
            || (edge_ids.len() == current.edge_ids.len()
                && edge_keys < current.edge_keys.as_slice())
    });
    if replace {
        *best = Some(Candidate {
            node_ids: node_ids.to_vec(),
            edge_ids: edge_ids.to_vec(),
            edge_keys: edge_keys.to_vec(),
        });
    }
}

fn is_terminal_kind(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::ExternalPackage
            | NodeKind::ExternalSystem
            | NodeKind::ExternalService
            | NodeKind::Unresolved
    )
}

fn edge_order(left: &Edge, right: &Edge) -> std::cmp::Ordering {
    EdgeKey::from(left).cmp(&EdgeKey::from(right))
}

impl From<&Edge> for EdgeKey {
    fn from(edge: &Edge) -> Self {
        Self {
            path: edge.evidence.path.clone(),
            line_start: edge.evidence.line_start,
            id: edge.id.clone(),
        }
    }
}

fn stable_id(prefix: &str, key: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{prefix}{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Evidence, Graph, Node, ScanSummary};

    #[test]
    fn longest_extracted_path_wins_over_an_earlier_leaf() {
        let graph = graph(
            &[
                node("A", NodeKind::Entry),
                node("B", NodeKind::Module),
                node("C", NodeKind::Module),
                node("D", NodeKind::ExternalPackage),
            ],
            &[
                edge("short", "A", "D", "a.ts", 1, Provenance::Extracted),
                edge("long-1", "A", "B", "z.ts", 2, Provenance::Extracted),
                edge("long-2", "B", "C", "z.ts", 3, Provenance::Extracted),
                edge("long-3", "C", "D", "z.ts", 4, Provenance::Extracted),
            ],
        );

        let flow = selected_static_path(&graph, "A").expect("path should exist");

        assert_eq!(flow.node_ids, ["A", "B", "C", "D"]);
        assert_eq!(flow.edge_ids, ["long-1", "long-2", "long-3"]);
        assert_eq!(flow.provenance, Provenance::Extracted);
    }

    #[test]
    fn evidence_order_breaks_equal_length_ties_regardless_of_vector_order() {
        let nodes = [
            node("A", NodeKind::Entry),
            node("B", NodeKind::Module),
            node("C", NodeKind::Module),
        ];
        let preferred = edge("later-id", "A", "B", "a.ts", 8, Provenance::Extracted);
        let other = edge("earlier-id", "A", "C", "b.ts", 1, Provenance::Extracted);
        let first = graph(&nodes, &[other.clone(), preferred.clone()]);
        let second = graph(&nodes, &[preferred, other]);

        let first_flow = selected_static_path(&first, "A").unwrap();
        let second_flow = selected_static_path(&second, "A").unwrap();

        assert_eq!(first_flow.edge_ids, ["later-id"]);
        assert_eq!(second_flow.edge_ids, first_flow.edge_ids);
        assert_eq!(second_flow.id, first_flow.id);
    }

    #[test]
    fn inferred_edges_are_ignored_and_external_nodes_end_the_path() {
        let graph = graph(
            &[
                node("A", NodeKind::Entry),
                node("B", NodeKind::ExternalService),
                node("C", NodeKind::Module),
            ],
            &[
                edge("real", "A", "B", "main.py", 2, Provenance::Extracted),
                edge(
                    "beyond-terminal",
                    "B",
                    "C",
                    "client.py",
                    7,
                    Provenance::Extracted,
                ),
                edge("guess", "A", "C", "main.py", 1, Provenance::Inferred),
            ],
        );

        let flow = selected_static_path(&graph, "A").unwrap();

        assert_eq!(flow.node_ids, ["A", "B"]);
        assert_eq!(flow.edge_ids, ["real"]);
    }

    #[test]
    fn cycle_closing_edge_is_literal_but_traversal_stops_on_the_repeat() {
        let graph = graph(
            &[node("A", NodeKind::Entry), node("B", NodeKind::Module)],
            &[
                edge("out", "A", "B", "a.ts", 1, Provenance::Extracted),
                edge("back", "B", "A", "b.ts", 1, Provenance::Extracted),
            ],
        );

        let flow = selected_static_path(&graph, "A").unwrap();

        assert_eq!(flow.node_ids, ["A", "B", "A"]);
        assert_eq!(flow.edge_ids, ["out", "back"]);
    }

    #[test]
    fn traversal_never_exceeds_twelve_hops() {
        let nodes: Vec<_> = (0..=13)
            .map(|index| node(&format!("N{index}"), NodeKind::Module))
            .collect();
        let edges: Vec<_> = (0..13)
            .map(|index| {
                edge(
                    &format!("E{index}"),
                    &format!("N{index}"),
                    &format!("N{}", index + 1),
                    "chain.ts",
                    index + 1,
                    Provenance::Extracted,
                )
            })
            .collect();
        let graph = graph(&nodes, &edges);

        let selection = selected_static_path_with_summary(&graph, "N0");
        let flow = selection.flow.unwrap();

        assert_eq!(flow.edge_ids.len(), 12);
        assert_eq!(flow.node_ids.len(), 13);
        assert_eq!(flow.node_ids.last().unwrap(), "N12");
        assert!(selection.search.hop_limit_reached);
        assert!(!selection.search.edge_exploration_limit_reached);
    }

    #[test]
    fn dense_graph_search_obeys_a_strict_deterministic_exploration_budget() {
        let nodes: Vec<_> = (0..40)
            .map(|index| node(&format!("N{index:02}"), NodeKind::Module))
            .collect();
        let mut edges = Vec::new();
        'sources: for source in 0..40 {
            for target in 0..40 {
                if source == target {
                    continue;
                }
                edges.push(edge(
                    &format!("E{source:02}-{target:02}"),
                    &format!("N{source:02}"),
                    &format!("N{target:02}"),
                    &format!("src/N{source:02}.ts"),
                    target + 1,
                    Provenance::Extracted,
                ));
                if edges.len() == 400 {
                    break 'sources;
                }
            }
        }
        let first = graph(&nodes, &edges);
        edges.reverse();
        let permuted = graph(&nodes, &edges);

        let first_selection = selected_static_path_bounded(&first, "N00", 7);
        let second_selection = selected_static_path_bounded(&permuted, "N00", 7);

        assert_eq!(first_selection.search.edge_explorations, 7);
        assert_eq!(second_selection.search.edge_explorations, 7);
        assert!(first_selection.search.edge_exploration_limit_reached);
        assert!(second_selection.search.edge_exploration_limit_reached);
        assert_eq!(
            first_selection.flow.unwrap().edge_ids,
            second_selection.flow.unwrap().edge_ids
        );
    }

    #[test]
    fn missing_leaf_and_terminal_selections_do_not_create_empty_flows() {
        let graph = graph(
            &[
                node("leaf", NodeKind::Module),
                node("outside", NodeKind::Unresolved),
            ],
            &[],
        );

        assert!(selected_static_path(&graph, "missing").is_none());
        assert!(selected_static_path(&graph, "leaf").is_none());
        assert!(selected_static_path(&graph, "outside").is_none());
    }

    #[test]
    fn identity_and_label_are_stable_and_selected_node_specific() {
        let graph = graph(
            &[
                node("A", NodeKind::Module),
                node("B", NodeKind::Module),
                node("C", NodeKind::Module),
            ],
            &[
                edge("AB", "A", "B", "a.ts", 1, Provenance::Extracted),
                edge("BC", "B", "C", "b.ts", 1, Provenance::Extracted),
            ],
        );

        let from_a = selected_static_path(&graph, "A").unwrap();
        let from_a_again = selected_static_path(&graph, "A").unwrap();
        let from_b = selected_static_path(&graph, "B").unwrap();

        assert_eq!(from_a.id, from_a_again.id);
        assert_eq!(from_a.label, from_a_again.label);
        assert_ne!(from_a.id, from_b.id);
        assert_ne!(from_a.label, from_b.label);
        assert!(from_a.label.contains("A"));
    }

    #[test]
    fn equal_length_paths_receive_distinct_content_ids() {
        let graph = graph(
            &[
                node("A", NodeKind::Module),
                node("B", NodeKind::Module),
                node("C", NodeKind::Module),
                node("D", NodeKind::Module),
            ],
            &[
                edge("AB", "A", "B", "a.ts", 1, Provenance::Extracted),
                edge("CD", "C", "D", "c.ts", 1, Provenance::Extracted),
            ],
        );

        let from_a = selected_static_path(&graph, "A").unwrap();
        let from_c = selected_static_path(&graph, "C").unwrap();

        assert_ne!(from_a.id, from_c.id);
    }

    #[test]
    fn path_identity_does_not_depend_on_checkout_directory_name() {
        let mut first = graph(
            &[node("A", NodeKind::Module), node("B", NodeKind::Module)],
            &[edge("AB", "A", "B", "a.ts", 1, Provenance::Extracted)],
        );
        let mut renamed = first.clone();
        first.repository = "first-checkout-name".into();
        renamed.repository = "renamed-checkout".into();

        let first_flow = selected_static_path(&first, "A").unwrap();
        let renamed_flow = selected_static_path(&renamed, "A").unwrap();

        assert_eq!(first_flow.id, renamed_flow.id);
    }

    fn graph(nodes: &[Node], edges: &[Edge]) -> Graph {
        Graph {
            schema_version: 2,
            repository: "test-repository".into(),
            nodes: nodes.to_vec(),
            edges: edges.to_vec(),
            flows: Vec::new(),
            scan_summary: ScanSummary {
                source: "test".into(),
                files_discovered: 0,
                files_scanned: 0,
                files_skipped: 0,
                skipped_by_reason: BTreeMap::new(),
                parse_warnings: 0,
                traversal_errors: 0,
                inferred_edges: 0,
            },
        }
    }

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            group: "TEST".into(),
            label: id.into(),
            kind,
            evidence: Evidence {
                path: format!("{id}.ts"),
                line_start: 1,
                line_end: 1,
            },
        }
    }

    fn edge(
        id: &str,
        source: &str,
        target: &str,
        path: &str,
        line: impl TryInto<u32>,
        provenance: Provenance,
    ) -> Edge {
        let line = line.try_into().ok().expect("test line should fit");
        Edge {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            relationship: "imports".into(),
            provenance,
            confidence: 1.0,
            evidence: Evidence {
                path: path.into(),
                line_start: line,
                line_end: line,
            },
            import_specifier: None,
        }
    }
}

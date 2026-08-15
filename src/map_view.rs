use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Flow, Graph, Node, NodeKind, Provenance};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MapMode {
    Overview,
    Focus,
    Trace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GroupLinkSummary {
    pub(crate) source_group: String,
    pub(crate) target_group: String,
    pub(crate) total: usize,
    pub(crate) inferred: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MapRenderPlan {
    pub(crate) visible_edge_ids: BTreeSet<String>,
    pub(crate) emphasized_node_ids: BTreeSet<String>,
    pub(crate) visible_label_ids: BTreeSet<String>,
    pub(crate) group_links: Vec<GroupLinkSummary>,
    pub(crate) internal_edge_counts: BTreeMap<String, usize>,
}

/// Returns honest static root candidates when the scanner found no explicit entry.
///
/// A candidate is an internal node with no discovered incoming relationship from
/// another internal node. The result is a graph-analysis aid, not a runtime or
/// launch claim. Explicit entries always take precedence and suppress candidates.
pub(crate) fn root_candidate_ids(graph: &Graph) -> Vec<String> {
    if graph
        .nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::Entry))
    {
        return Vec::new();
    }

    let internal_ids: BTreeSet<_> = graph
        .nodes
        .iter()
        .filter(|node| is_internal(node))
        .map(|node| node.id.as_str())
        .collect();
    let imported_internal_ids: BTreeSet<_> = graph
        .edges
        .iter()
        .filter(|edge| {
            internal_ids.contains(edge.source.as_str())
                && internal_ids.contains(edge.target.as_str())
        })
        .map(|edge| edge.target.as_str())
        .collect();

    let mut candidates: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            internal_ids.contains(node.id.as_str())
                && !imported_internal_ids.contains(node.id.as_str())
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.evidence
            .path
            .cmp(&right.evidence.path)
            .then_with(|| left.id.cmp(&right.id))
    });
    candidates.into_iter().map(|node| node.id.clone()).collect()
}

pub(crate) fn build_render_plan(
    graph: &Graph,
    mode: MapMode,
    selected_id: Option<&str>,
    trace: Option<&Flow>,
) -> MapRenderPlan {
    match mode {
        MapMode::Overview => overview_plan(graph),
        MapMode::Focus => focus_plan(graph, selected_id),
        MapMode::Trace => trace_plan(graph, trace),
    }
}

fn overview_plan(graph: &Graph) -> MapRenderPlan {
    let nodes_by_id: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let mut links: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    let mut internal_edge_counts = BTreeMap::new();

    for edge in &graph.edges {
        let (Some(source), Some(target)) = (
            nodes_by_id.get(edge.source.as_str()),
            nodes_by_id.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if source.group == target.group {
            *internal_edge_counts
                .entry(source.group.clone())
                .or_insert(0) += 1;
            continue;
        }
        let counts = links
            .entry((source.group.clone(), target.group.clone()))
            .or_insert((0, 0));
        counts.0 += 1;
        if edge.provenance == Provenance::Inferred {
            counts.1 += 1;
        }
    }

    let emphasized_node_ids: BTreeSet<_> = overview_anchor_ids(graph).into_iter().collect();
    MapRenderPlan {
        visible_edge_ids: BTreeSet::new(),
        visible_label_ids: emphasized_node_ids.clone(),
        emphasized_node_ids,
        group_links: links
            .into_iter()
            .map(
                |((source_group, target_group), (total, inferred))| GroupLinkSummary {
                    source_group,
                    target_group,
                    total,
                    inferred,
                },
            )
            .collect(),
        internal_edge_counts,
    }
}

fn focus_plan(graph: &Graph, selected_id: Option<&str>) -> MapRenderPlan {
    let Some(selected_id) =
        selected_id.filter(|selected| graph.nodes.iter().any(|node| node.id.as_str() == *selected))
    else {
        return MapRenderPlan::default();
    };

    let known_node_ids: BTreeSet<_> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut plan = MapRenderPlan::default();
    plan.emphasized_node_ids.insert(selected_id.to_owned());
    for edge in &graph.edges {
        if edge.source != selected_id && edge.target != selected_id {
            continue;
        }
        plan.visible_edge_ids.insert(edge.id.clone());
        if known_node_ids.contains(edge.source.as_str()) {
            plan.emphasized_node_ids.insert(edge.source.clone());
        }
        if known_node_ids.contains(edge.target.as_str()) {
            plan.emphasized_node_ids.insert(edge.target.clone());
        }
    }
    plan.visible_label_ids = plan.emphasized_node_ids.clone();
    plan
}

fn trace_plan(graph: &Graph, trace: Option<&Flow>) -> MapRenderPlan {
    let Some(trace) = trace else {
        return MapRenderPlan::default();
    };
    let nodes_by_id: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let edges_by_id: BTreeMap<_, _> = graph
        .edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge))
        .collect();
    let mut plan = MapRenderPlan::default();

    for node_id in &trace.node_ids {
        if nodes_by_id.contains_key(node_id.as_str()) {
            plan.emphasized_node_ids.insert(node_id.clone());
        }
    }
    for edge_id in &trace.edge_ids {
        let Some(edge) = edges_by_id.get(edge_id.as_str()) else {
            continue;
        };
        plan.visible_edge_ids.insert(edge.id.clone());
        if nodes_by_id.contains_key(edge.source.as_str()) {
            plan.emphasized_node_ids.insert(edge.source.clone());
        }
        if nodes_by_id.contains_key(edge.target.as_str()) {
            plan.emphasized_node_ids.insert(edge.target.clone());
        }
    }
    plan.visible_label_ids = plan.emphasized_node_ids.clone();
    plan
}

fn overview_anchor_ids(graph: &Graph) -> Vec<String> {
    let mut entries: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Entry))
        .collect();
    entries.sort_by(|left, right| {
        left.evidence
            .path
            .cmp(&right.evidence.path)
            .then_with(|| left.id.cmp(&right.id))
    });
    if entries.is_empty() {
        root_candidate_ids(graph)
    } else {
        entries.into_iter().map(|node| node.id.clone()).collect()
    }
}

fn is_internal(node: &Node) -> bool {
    !matches!(
        node.kind,
        NodeKind::ExternalPackage
            | NodeKind::ExternalSystem
            | NodeKind::ExternalService
            | NodeKind::Unresolved
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Edge, Evidence, Graph, ScanSummary};

    #[test]
    fn explicit_entries_take_precedence_over_root_candidates() {
        let graph = graph(
            vec![
                node("entry", "src", NodeKind::Entry, "src/main.ts"),
                node("unused", "src", NodeKind::Module, "src/unused.ts"),
            ],
            Vec::new(),
        );

        assert!(root_candidate_ids(&graph).is_empty());
        let plan = build_render_plan(&graph, MapMode::Overview, None, None);
        assert_eq!(plan.emphasized_node_ids, BTreeSet::from(["entry".into()]));
        assert_eq!(plan.visible_label_ids, plan.emphasized_node_ids);
    }

    #[test]
    fn root_candidates_exclude_boundaries_and_sort_by_evidence_then_id() {
        let graph = graph(
            vec![
                node("later", "src", NodeKind::Module, "z.ts"),
                node("same-b", "src", NodeKind::Module, "a.ts"),
                node("same-a", "src", NodeKind::Module, "a.ts"),
                node(
                    "package",
                    "EXTERNAL",
                    NodeKind::ExternalPackage,
                    "package.json",
                ),
                node("unresolved", "UNRESOLVED", NodeKind::Unresolved, "a.ts"),
            ],
            Vec::new(),
        );

        assert_eq!(root_candidate_ids(&graph), ["same-a", "same-b", "later"]);
    }

    #[test]
    fn root_candidates_are_stable_under_input_permutation() {
        let graph = graph(
            vec![
                node("cli", "src", NodeKind::Module, "src/cli.ts"),
                node("agent", "src", NodeKind::Module, "src/agent.ts"),
                node("logger", "src", NodeKind::Module, "src/logger.ts"),
            ],
            vec![
                edge("cli-agent", "cli", "agent", Provenance::Extracted),
                edge("agent-logger", "agent", "logger", Provenance::Extracted),
            ],
        );
        let mut permuted = graph.clone();
        permuted.nodes.reverse();
        permuted.edges.reverse();

        assert_eq!(root_candidate_ids(&graph), ["cli"]);
        assert_eq!(root_candidate_ids(&permuted), root_candidate_ids(&graph));
        assert_eq!(
            build_render_plan(&permuted, MapMode::Overview, None, None),
            build_render_plan(&graph, MapMode::Overview, None, None)
        );
    }

    #[test]
    fn a_closed_internal_cycle_has_no_root_candidate() {
        let graph = graph(
            vec![
                node("a", "src", NodeKind::Module, "a.ts"),
                node("b", "src", NodeKind::Module, "b.ts"),
            ],
            vec![
                edge("a-b", "a", "b", Provenance::Extracted),
                edge("b-a", "b", "a", Provenance::Extracted),
            ],
        );

        assert!(root_candidate_ids(&graph).is_empty());
        let plan = build_render_plan(&graph, MapMode::Overview, None, None);
        assert!(plan.emphasized_node_ids.is_empty());
        assert_eq!(plan.internal_edge_counts["src"], 2);
    }

    #[test]
    fn overview_aggregates_dogfood_like_boundaries_without_module_edges() {
        let graph = graph(
            vec![
                node("cli", "src", NodeKind::Module, "src/cli.ts"),
                node("agent", "src", NodeKind::Module, "src/agent.ts"),
                node("fetch", "tools", NodeKind::Module, "src/tools/fetch.ts"),
                node(
                    "package",
                    "EXTERNAL",
                    NodeKind::ExternalPackage,
                    "package.json",
                ),
            ],
            vec![
                edge("src-internal", "cli", "agent", Provenance::Extracted),
                edge("src-tools", "agent", "fetch", Provenance::Extracted),
                edge("tools-src", "fetch", "agent", Provenance::Extracted),
                edge("src-ext-real", "agent", "package", Provenance::Extracted),
                edge("src-ext-guess", "cli", "package", Provenance::Inferred),
                edge("tools-ext-guess", "fetch", "package", Provenance::Inferred),
            ],
        );

        let plan = build_render_plan(&graph, MapMode::Overview, Some("agent"), None);

        assert!(plan.visible_edge_ids.is_empty());
        assert_eq!(plan.emphasized_node_ids, BTreeSet::from(["cli".into()]));
        assert_eq!(
            plan.internal_edge_counts,
            BTreeMap::from([("src".into(), 1)])
        );
        assert_eq!(
            plan.group_links,
            [
                GroupLinkSummary {
                    source_group: "src".into(),
                    target_group: "EXTERNAL".into(),
                    total: 2,
                    inferred: 1,
                },
                GroupLinkSummary {
                    source_group: "src".into(),
                    target_group: "tools".into(),
                    total: 1,
                    inferred: 0,
                },
                GroupLinkSummary {
                    source_group: "tools".into(),
                    target_group: "EXTERNAL".into(),
                    total: 1,
                    inferred: 1,
                },
                GroupLinkSummary {
                    source_group: "tools".into(),
                    target_group: "src".into(),
                    total: 1,
                    inferred: 0,
                },
            ]
        );
    }

    #[test]
    fn focus_contains_only_selected_incident_edges_and_known_endpoints() {
        let mut graph = graph(
            vec![
                node("a", "src", NodeKind::Module, "a.ts"),
                node("b", "src", NodeKind::Module, "b.ts"),
                node("c", "src", NodeKind::Module, "c.ts"),
                node("d", "src", NodeKind::Module, "d.ts"),
            ],
            vec![
                edge("incoming", "a", "b", Provenance::Extracted),
                edge("outgoing", "b", "c", Provenance::Extracted),
                edge("unrelated", "c", "d", Provenance::Extracted),
            ],
        );
        graph.edges.push(edge(
            "missing-endpoint",
            "b",
            "missing",
            Provenance::Extracted,
        ));

        let plan = build_render_plan(&graph, MapMode::Focus, Some("b"), None);

        assert_eq!(
            plan.visible_edge_ids,
            BTreeSet::from([
                "incoming".into(),
                "missing-endpoint".into(),
                "outgoing".into()
            ])
        );
        assert_eq!(
            plan.emphasized_node_ids,
            BTreeSet::from(["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(plan.visible_label_ids, plan.emphasized_node_ids);
        assert!(plan.group_links.is_empty());
        assert!(plan.internal_edge_counts.is_empty());
        assert_eq!(
            build_render_plan(&graph, MapMode::Focus, Some("unknown"), None),
            MapRenderPlan::default()
        );
    }

    #[test]
    fn trace_contains_only_contract_edges_and_nodes_that_exist_in_graph() {
        let graph = graph(
            vec![
                node("a", "src", NodeKind::Module, "a.ts"),
                node("b", "src", NodeKind::Module, "b.ts"),
                node("c", "src", NodeKind::Module, "c.ts"),
            ],
            vec![
                edge("path", "a", "b", Provenance::Extracted),
                edge("unrelated", "b", "c", Provenance::Extracted),
            ],
        );
        let trace = Flow {
            id: "flow".into(),
            label: "static path".into(),
            provenance: Provenance::Extracted,
            node_ids: vec!["a".into(), "missing".into()],
            edge_ids: vec!["path".into(), "missing-edge".into()],
        };

        let plan = build_render_plan(&graph, MapMode::Trace, Some("c"), Some(&trace));

        assert_eq!(plan.visible_edge_ids, BTreeSet::from(["path".into()]));
        assert_eq!(
            plan.emphasized_node_ids,
            BTreeSet::from(["a".into(), "b".into()])
        );
        assert_eq!(plan.visible_label_ids, plan.emphasized_node_ids);
        assert!(plan.group_links.is_empty());
        assert!(plan.internal_edge_counts.is_empty());
    }

    fn graph(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph {
            schema_version: 2,
            repository: "TEST".into(),
            nodes,
            edges,
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

    fn node(id: &str, group: &str, kind: NodeKind, path: &str) -> Node {
        Node {
            id: id.into(),
            group: group.into(),
            label: id.into(),
            kind,
            evidence: Evidence {
                path: path.into(),
                line_start: 1,
                line_end: 1,
            },
        }
    }

    fn edge(id: &str, source: &str, target: &str, provenance: Provenance) -> Edge {
        Edge {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            relationship: "imports".into(),
            provenance,
            confidence: if provenance == Provenance::Extracted {
                1.0
            } else {
                0.72
            },
            evidence: Evidence {
                path: format!("{source}.ts"),
                line_start: 1,
                line_end: 1,
            },
            import_specifier: None,
        }
    }
}

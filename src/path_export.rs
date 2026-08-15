use std::{collections::BTreeSet, error::Error, fmt, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    model::{Edge, Flow, Graph, Node, NodeKind, ScanSummary},
    path_explorer::{PathSearchSummary, selected_static_path_with_summary},
};

/// Schema version for the portable selected-path export contract.
pub const PATH_EXPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathArtifactType {
    StaticPath,
}

pub const PATH_EXPORT_ARTIFACT_TYPE: PathArtifactType = PathArtifactType::StaticPath;

/// A self-contained, portable view of one evidence-backed static path.
///
/// Nodes and edges appear in path order and include only records referenced by
/// `flow`. Evidence paths are guaranteed to be repository-relative when the
/// value is constructed through [`export_selected_static_path`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathExport {
    pub artifact_type: PathArtifactType,
    pub schema_version: u32,
    pub source_graph_schema_version: u32,
    pub repository: String,
    pub selector: String,
    pub flow: Flow,
    pub path_search: PathSearchSummary,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub scan_summary: ScanSummary,
}

/// Builds a portable export for the local node whose evidence path exactly
/// matches `module_path`.
///
/// External and unresolved nodes are not module selectors, even though their
/// evidence points back to the importing source file. This keeps a normal
/// source path from becoming ambiguous merely because that source imports
/// external packages.
pub fn export_selected_static_path(
    graph: &Graph,
    module_path: &str,
) -> Result<PathExport, PathExportError> {
    validate_unique_ids(graph)?;
    let selector = normalize_selector(module_path)?;
    let mut matches: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| is_local_node(&node.kind) && node.evidence.path == selector)
        .collect();
    matches.sort_by(|left, right| left.id.cmp(&right.id));

    let selected = match matches.as_slice() {
        [] => {
            return Err(PathExportError::SelectorNotFound { selector });
        }
        [selected] => *selected,
        _ => {
            return Err(PathExportError::AmbiguousSelector {
                selector,
                matching_node_ids: matches.into_iter().map(|node| node.id.clone()).collect(),
            });
        }
    };

    validate_evidence_path("node", &selected.id, &selected.evidence.path)?;

    let selection = selected_static_path_with_summary(graph, &selected.id);
    let flow = selection
        .flow
        .ok_or_else(|| PathExportError::NoOutwardPath {
            selector: selector.clone(),
            node_id: selected.id.clone(),
        })?;

    let nodes = referenced_nodes(graph, &flow)?;
    let edges = referenced_edges(graph, &flow)?;
    validate_evidence(&nodes, &edges)?;
    validate_import_specifiers(&edges)?;
    validate_flow(&flow, &edges)?;

    Ok(PathExport {
        artifact_type: PATH_EXPORT_ARTIFACT_TYPE,
        schema_version: PATH_EXPORT_SCHEMA_VERSION,
        source_graph_schema_version: graph.schema_version,
        repository: graph.repository.clone(),
        selector,
        flow,
        path_search: selection.search,
        nodes,
        edges,
        scan_summary: graph.scan_summary.clone(),
    })
}

fn referenced_nodes(graph: &Graph, flow: &Flow) -> Result<Vec<Node>, PathExportError> {
    let mut seen = BTreeSet::new();
    flow.node_ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .map(|id| {
            graph
                .nodes
                .iter()
                .find(|node| node.id == *id)
                .cloned()
                .ok_or_else(|| PathExportError::MissingGraphReference {
                    record_kind: "node",
                    record_id: id.clone(),
                })
        })
        .collect()
}

fn referenced_edges(graph: &Graph, flow: &Flow) -> Result<Vec<Edge>, PathExportError> {
    let mut seen = BTreeSet::new();
    flow.edge_ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .map(|id| {
            graph
                .edges
                .iter()
                .find(|edge| edge.id == *id)
                .cloned()
                .ok_or_else(|| PathExportError::MissingGraphReference {
                    record_kind: "edge",
                    record_id: id.clone(),
                })
        })
        .collect()
}

fn validate_unique_ids(graph: &Graph) -> Result<(), PathExportError> {
    let mut node_ids = BTreeSet::new();
    for node in &graph.nodes {
        if !node_ids.insert(node.id.as_str()) {
            return Err(PathExportError::DuplicateRecordId {
                record_kind: "node",
                record_id: node.id.clone(),
            });
        }
    }
    let mut edge_ids = BTreeSet::new();
    for edge in &graph.edges {
        if !edge_ids.insert(edge.id.as_str()) {
            return Err(PathExportError::DuplicateRecordId {
                record_kind: "edge",
                record_id: edge.id.clone(),
            });
        }
    }
    Ok(())
}

fn normalize_selector(selector: &str) -> Result<String, PathExportError> {
    if selector.is_empty() || is_absolute_on_any_platform(selector) {
        return Err(PathExportError::InvalidSelector {
            selector: selector.to_owned(),
        });
    }
    let mut normalized = Vec::new();
    for component in selector.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                return Err(PathExportError::InvalidSelector {
                    selector: selector.to_owned(),
                });
            }
            component => normalized.push(component),
        }
    }
    if normalized.is_empty() {
        return Err(PathExportError::InvalidSelector {
            selector: selector.to_owned(),
        });
    }
    Ok(normalized.join("/"))
}

fn validate_evidence(nodes: &[Node], edges: &[Edge]) -> Result<(), PathExportError> {
    for node in nodes {
        validate_evidence_path("node", &node.id, &node.evidence.path)?;
        validate_evidence_range(
            "node",
            &node.id,
            node.evidence.line_start,
            node.evidence.line_end,
        )?;
    }
    for edge in edges {
        validate_evidence_path("edge", &edge.id, &edge.evidence.path)?;
        validate_evidence_range(
            "edge",
            &edge.id,
            edge.evidence.line_start,
            edge.evidence.line_end,
        )?;
    }
    Ok(())
}

fn validate_evidence_range(
    kind: &'static str,
    id: &str,
    line_start: u32,
    line_end: u32,
) -> Result<(), PathExportError> {
    if line_start == 0 || line_end < line_start {
        return Err(PathExportError::InvalidEvidenceRange {
            record_kind: kind,
            record_id: id.to_owned(),
            line_start,
            line_end,
        });
    }
    Ok(())
}

fn validate_flow(flow: &Flow, edges: &[Edge]) -> Result<(), PathExportError> {
    if flow.node_ids.len() != flow.edge_ids.len() + 1 {
        return Err(PathExportError::InvalidFlow {
            detail: "node count must equal edge count plus one".into(),
        });
    }
    for (index, edge_id) in flow.edge_ids.iter().enumerate() {
        let edge = edges
            .iter()
            .find(|edge| edge.id == *edge_id)
            .ok_or_else(|| PathExportError::MissingGraphReference {
                record_kind: "edge",
                record_id: edge_id.clone(),
            })?;
        if edge.provenance != crate::model::Provenance::Extracted {
            return Err(PathExportError::InvalidFlow {
                detail: format!("edge `{edge_id}` is not extracted"),
            });
        }
        if edge.source != flow.node_ids[index] || edge.target != flow.node_ids[index + 1] {
            return Err(PathExportError::InvalidFlow {
                detail: format!("edge `{edge_id}` does not join its adjacent flow nodes"),
            });
        }
    }
    Ok(())
}

fn validate_import_specifiers(edges: &[Edge]) -> Result<(), PathExportError> {
    for edge in edges {
        if edge
            .import_specifier
            .as_deref()
            .is_some_and(is_absolute_on_any_platform)
        {
            return Err(PathExportError::AbsoluteImportSpecifier {
                edge_id: edge.id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_evidence_path(kind: &'static str, id: &str, path: &str) -> Result<(), PathExportError> {
    if path.is_empty() {
        return Err(PathExportError::EmptyEvidencePath {
            record_kind: kind,
            record_id: id.to_owned(),
        });
    }
    if is_absolute_on_any_platform(path) {
        return Err(PathExportError::AbsoluteEvidencePath {
            record_kind: kind,
            record_id: id.to_owned(),
            path: path.to_owned(),
        });
    }
    if path.split(['/', '\\']).any(|component| component == "..") {
        return Err(PathExportError::EscapingEvidencePath {
            record_kind: kind,
            record_id: id.to_owned(),
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn is_absolute_on_any_platform(path: &str) -> bool {
    if Path::new(path).is_absolute()
        || path.starts_with('\\')
        || path
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
    {
        return true;
    }

    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_local_node(kind: &NodeKind) -> bool {
    !matches!(
        kind,
        NodeKind::ExternalPackage
            | NodeKind::ExternalSystem
            | NodeKind::ExternalService
            | NodeKind::Unresolved
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathExportError {
    InvalidSelector {
        selector: String,
    },
    SelectorNotFound {
        selector: String,
    },
    AmbiguousSelector {
        selector: String,
        matching_node_ids: Vec<String>,
    },
    NoOutwardPath {
        selector: String,
        node_id: String,
    },
    AbsoluteEvidencePath {
        record_kind: &'static str,
        record_id: String,
        path: String,
    },
    EscapingEvidencePath {
        record_kind: &'static str,
        record_id: String,
        path: String,
    },
    EmptyEvidencePath {
        record_kind: &'static str,
        record_id: String,
    },
    DuplicateRecordId {
        record_kind: &'static str,
        record_id: String,
    },
    MissingGraphReference {
        record_kind: &'static str,
        record_id: String,
    },
    InvalidEvidenceRange {
        record_kind: &'static str,
        record_id: String,
        line_start: u32,
        line_end: u32,
    },
    InvalidFlow {
        detail: String,
    },
    AbsoluteImportSpecifier {
        edge_id: String,
    },
}

impl fmt::Display for PathExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSelector { selector } => write!(
                formatter,
                "module path `{selector}` must be a non-empty repository-relative source path"
            ),
            Self::SelectorNotFound { selector } => write!(
                formatter,
                "module path `{selector}` does not match a scanned local node"
            ),
            Self::AmbiguousSelector {
                selector,
                matching_node_ids,
            } => write!(
                formatter,
                "module path `{selector}` is ambiguous; matching node IDs: {}",
                matching_node_ids.join(", ")
            ),
            Self::NoOutwardPath { selector, node_id } => write!(
                formatter,
                "module path `{selector}` (node `{node_id}`) has no outward extracted static path"
            ),
            Self::AbsoluteEvidencePath {
                record_kind,
                record_id,
                path,
            } => write!(
                formatter,
                "{record_kind} `{record_id}` has non-portable absolute evidence path `{path}`"
            ),
            Self::EscapingEvidencePath {
                record_kind,
                record_id,
                path,
            } => write!(
                formatter,
                "{record_kind} `{record_id}` has evidence path `{path}` that escapes the repository"
            ),
            Self::EmptyEvidencePath {
                record_kind,
                record_id,
            } => write!(
                formatter,
                "{record_kind} `{record_id}` has an empty evidence path"
            ),
            Self::DuplicateRecordId {
                record_kind,
                record_id,
            } => write!(
                formatter,
                "graph contains duplicate {record_kind} ID `{record_id}`"
            ),
            Self::MissingGraphReference {
                record_kind,
                record_id,
            } => write!(
                formatter,
                "selected path references missing {record_kind} `{record_id}`"
            ),
            Self::InvalidEvidenceRange {
                record_kind,
                record_id,
                line_start,
                line_end,
            } => write!(
                formatter,
                "{record_kind} `{record_id}` has invalid evidence lines {line_start}..{line_end}"
            ),
            Self::InvalidFlow { detail } => {
                write!(
                    formatter,
                    "selected path is internally inconsistent: {detail}"
                )
            }
            Self::AbsoluteImportSpecifier { edge_id } => write!(
                formatter,
                "edge `{edge_id}` contains an absolute source import; refusing portable export"
            ),
        }
    }
}

impl Error for PathExportError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::{Evidence, Provenance};

    #[test]
    fn export_contains_only_path_records_in_path_order() {
        let graph = graph(
            vec![
                node("unused", "src/unused.ts", NodeKind::Module),
                node("leaf", "src/leaf.ts", NodeKind::Module),
                node("entry", "src/main.ts", NodeKind::Entry),
                node("middle", "src/middle.ts", NodeKind::Module),
            ],
            vec![
                edge("unused-edge", "entry", "unused", "src/main.ts", 8),
                edge("first", "entry", "middle", "src/main.ts", 2),
                edge("second", "middle", "leaf", "src/middle.ts", 3),
            ],
        );

        let export = export_selected_static_path(&graph, "src/main.ts").unwrap();

        assert_eq!(export.schema_version, PATH_EXPORT_SCHEMA_VERSION);
        assert_eq!(export.artifact_type, PATH_EXPORT_ARTIFACT_TYPE);
        assert_eq!(export.source_graph_schema_version, 2);
        assert_eq!(export.selector, "src/main.ts");
        assert_eq!(export.path_search.max_hops, 12);
        assert_eq!(export.path_search.max_edge_explorations, 20_000);
        assert!(!export.path_search.hop_limit_reached);
        assert!(!export.path_search.edge_exploration_limit_reached);
        assert_eq!(
            export
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["entry", "middle", "leaf"]
        );
        assert_eq!(
            export
                .edges
                .iter()
                .map(|edge| edge.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(export.flow.node_ids, ["entry", "middle", "leaf"]);
        assert_eq!(export.flow.edge_ids, ["first", "second"]);
    }

    #[test]
    fn pretty_json_is_identical_when_graph_vectors_are_permuted() {
        let nodes = vec![
            node("entry", "src/main.py", NodeKind::Entry),
            node("alpha", "src/alpha.py", NodeKind::Module),
            node("omega", "src/omega.py", NodeKind::Module),
        ];
        let edges = vec![
            edge("preferred", "entry", "alpha", "src/main.py", 1),
            edge("other", "entry", "omega", "src/main.py", 2),
        ];
        let first = graph(nodes.clone(), edges.clone());
        let mut reversed_nodes = nodes;
        reversed_nodes.reverse();
        let mut reversed_edges = edges;
        reversed_edges.reverse();
        let second = graph(reversed_nodes, reversed_edges);

        let first_json = serde_json::to_string_pretty(
            &export_selected_static_path(&first, "src/main.py").unwrap(),
        )
        .unwrap();
        let second_json = serde_json::to_string_pretty(
            &export_selected_static_path(&second, "src/main.py").unwrap(),
        )
        .unwrap();

        assert_eq!(first_json, second_json);
    }

    #[test]
    fn external_evidence_does_not_make_a_source_selector_ambiguous() {
        let mut graph = graph(
            vec![
                node("entry", "src/main.ts", NodeKind::Entry),
                node("leaf", "src/leaf.ts", NodeKind::Module),
                node("package", "src/main.ts", NodeKind::ExternalPackage),
            ],
            vec![edge("out", "entry", "leaf", "src/main.ts", 1)],
        );
        graph.nodes[2].label = "react".into();

        let export = export_selected_static_path(&graph, "src/main.ts").unwrap();

        assert_eq!(export.flow.node_ids.first().unwrap(), "entry");
    }

    #[test]
    fn selector_normalizes_dot_prefixes_and_windows_separators() {
        let graph = graph(
            vec![
                node("entry", "src/main.rs", NodeKind::Entry),
                node("leaf", "src/leaf.rs", NodeKind::Module),
            ],
            vec![edge("out", "entry", "leaf", "src/main.rs", 1)],
        );

        let dotted = export_selected_static_path(&graph, "./src/main.rs").unwrap();
        let windows = export_selected_static_path(&graph, r"src\main.rs").unwrap();

        assert_eq!(dotted.selector, "src/main.rs");
        assert_eq!(windows.selector, "src/main.rs");
        assert_eq!(
            serde_json::to_string_pretty(&dotted).unwrap(),
            serde_json::to_string_pretty(&windows).unwrap()
        );
    }

    #[test]
    fn unsafe_selectors_and_duplicate_graph_ids_are_typed_errors() {
        let empty_graph = graph(Vec::new(), Vec::new());
        for selector in [
            "",
            ".",
            "../src/main.rs",
            "/repo/src/main.rs",
            "C:src/main.rs",
            r"C:\repo\main.rs",
        ] {
            assert!(matches!(
                export_selected_static_path(&empty_graph, selector),
                Err(PathExportError::InvalidSelector { .. })
            ));
        }

        let duplicate_graph = graph(
            vec![
                node("same", "src/main.rs", NodeKind::Entry),
                node("same", "src/leaf.rs", NodeKind::Module),
            ],
            Vec::new(),
        );
        assert_eq!(
            export_selected_static_path(&duplicate_graph, "src/main.rs").unwrap_err(),
            PathExportError::DuplicateRecordId {
                record_kind: "node",
                record_id: "same".into(),
            }
        );
    }

    #[test]
    fn missing_flow_references_are_never_silently_dropped() {
        let graph = graph(
            vec![node("entry", "src/main.rs", NodeKind::Entry)],
            Vec::new(),
        );
        let flow = Flow {
            id: "path".into(),
            label: "STATIC PATH".into(),
            provenance: Provenance::Extracted,
            node_ids: vec!["entry".into(), "missing".into()],
            edge_ids: Vec::new(),
        };

        assert_eq!(
            referenced_nodes(&graph, &flow).unwrap_err(),
            PathExportError::MissingGraphReference {
                record_kind: "node",
                record_id: "missing".into(),
            }
        );
    }

    #[test]
    fn missing_ambiguous_and_leaf_selectors_are_typed_errors() {
        let graph = graph(
            vec![
                node("one", "src/shared.ts", NodeKind::Module),
                node("two", "src/shared.ts", NodeKind::Component),
                node("leaf", "src/leaf.ts", NodeKind::Module),
            ],
            Vec::new(),
        );

        assert_eq!(
            export_selected_static_path(&graph, "src/missing.ts").unwrap_err(),
            PathExportError::SelectorNotFound {
                selector: "src/missing.ts".into()
            }
        );
        assert_eq!(
            export_selected_static_path(&graph, "src/shared.ts").unwrap_err(),
            PathExportError::AmbiguousSelector {
                selector: "src/shared.ts".into(),
                matching_node_ids: vec!["one".into(), "two".into()]
            }
        );
        assert_eq!(
            export_selected_static_path(&graph, "src/leaf.ts").unwrap_err(),
            PathExportError::NoOutwardPath {
                selector: "src/leaf.ts".into(),
                node_id: "leaf".into()
            }
        );
        assert!(
            export_selected_static_path(&graph, "src/leaf.ts")
                .unwrap_err()
                .to_string()
                .contains("no outward extracted static path")
        );
    }

    #[test]
    fn absolute_and_escaping_evidence_are_rejected() {
        let absolute_node_graph = graph(
            vec![
                node("entry", "src/main.ts", NodeKind::Entry),
                node("leaf", "/private/leaf.ts", NodeKind::Module),
            ],
            vec![edge("out", "entry", "leaf", "src/main.ts", 1)],
        );
        assert!(matches!(
            export_selected_static_path(&absolute_node_graph, "src/main.ts"),
            Err(PathExportError::AbsoluteEvidencePath {
                record_kind: "node",
                record_id,
                ..
            }) if record_id == "leaf"
        ));

        let absolute_edge_graph = graph(
            vec![
                node("entry", "src/main.ts", NodeKind::Entry),
                node("leaf", "src/leaf.ts", NodeKind::Module),
            ],
            vec![edge("out", "entry", "leaf", r"C:\\repo\\src\\main.ts", 1)],
        );
        assert!(matches!(
            export_selected_static_path(&absolute_edge_graph, "src/main.ts"),
            Err(PathExportError::AbsoluteEvidencePath {
                record_kind: "edge",
                record_id,
                ..
            }) if record_id == "out"
        ));

        let escaping_graph = graph(
            vec![
                node("entry", "src/main.ts", NodeKind::Entry),
                node("leaf", "../leaf.ts", NodeKind::Module),
            ],
            vec![edge("out", "entry", "leaf", "src/main.ts", 1)],
        );
        assert!(matches!(
            export_selected_static_path(&escaping_graph, "src/main.ts"),
            Err(PathExportError::EscapingEvidencePath { record_id, .. }) if record_id == "leaf"
        ));

        let empty_graph = graph(
            vec![
                node("entry", "src/main.ts", NodeKind::Entry),
                node("leaf", "", NodeKind::Module),
            ],
            vec![edge("out", "entry", "leaf", "src/main.ts", 1)],
        );
        assert!(matches!(
            export_selected_static_path(&empty_graph, "src/main.ts"),
            Err(PathExportError::EmptyEvidencePath { record_id, .. }) if record_id == "leaf"
        ));
    }

    #[test]
    fn artifact_round_trips_and_rejects_unknown_fields() {
        let graph = graph(
            vec![
                node("entry", "src/main.ts", NodeKind::Entry),
                node("leaf", "src/leaf.ts", NodeKind::Module),
            ],
            vec![edge("out", "entry", "leaf", "src/main.ts", 1)],
        );
        let export = export_selected_static_path(&graph, "src/main.ts").unwrap();
        let value = serde_json::to_value(&export).unwrap();
        let decoded: PathExport = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.flow.id, export.flow.id);

        let mut with_unknown = value;
        with_unknown
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<PathExport>(with_unknown).is_err());

        let mut wrong_type = serde_json::to_value(&export).unwrap();
        wrong_type["artifact_type"] = serde_json::Value::String("graph".into());
        assert!(serde_json::from_value::<PathExport>(wrong_type).is_err());
    }

    #[test]
    fn invalid_evidence_line_ranges_are_rejected() {
        let mut graph = graph(
            vec![
                node("entry", "src/main.ts", NodeKind::Entry),
                node("leaf", "src/leaf.ts", NodeKind::Module),
            ],
            vec![edge("out", "entry", "leaf", "src/main.ts", 1)],
        );
        graph.edges[0].evidence.line_start = 0;

        assert!(matches!(
            export_selected_static_path(&graph, "src/main.ts"),
            Err(PathExportError::InvalidEvidenceRange {
                record_kind: "edge",
                record_id,
                ..
            }) if record_id == "out"
        ));
    }

    #[test]
    fn absolute_source_imports_are_refused_without_echoing_the_literal() {
        for specifier in [
            "/Users/example/private/token",
            r"C:\Users\example\private\token",
            "file:///Users/example/private/token",
        ] {
            let mut graph = graph(
                vec![
                    node("entry", "src/main.ts", NodeKind::Entry),
                    node("leaf", "src/main.ts", NodeKind::Unresolved),
                ],
                vec![edge("out", "entry", "leaf", "src/main.ts", 1)],
            );
            graph.edges[0].import_specifier = Some(specifier.into());

            let error = export_selected_static_path(&graph, "src/main.ts").unwrap_err();
            assert_eq!(
                error,
                PathExportError::AbsoluteImportSpecifier {
                    edge_id: "out".into(),
                }
            );
            assert!(!error.to_string().contains(specifier));
        }
    }

    fn graph(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph {
            schema_version: 2,
            repository: "portable-repository".into(),
            nodes,
            edges,
            flows: Vec::new(),
            scan_summary: ScanSummary {
                source: "test".into(),
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

    fn node(id: &str, path: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            group: "TEST".into(),
            label: id.into(),
            kind,
            evidence: Evidence {
                path: path.into(),
                line_start: 1,
                line_end: 1,
            },
        }
    }

    fn edge(id: &str, source: &str, target: &str, path: &str, line: u32) -> Edge {
        Edge {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            relationship: "imports".into(),
            provenance: Provenance::Extracted,
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

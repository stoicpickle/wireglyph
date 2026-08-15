use std::collections::{HashMap, HashSet};

use wireglyph::{fixture::load_beacon_ops, model::Provenance};

#[test]
fn beacon_ops_has_the_intended_spike_shape() {
    let graph = load_beacon_ops().expect("fixture JSON must satisfy the graph contract");

    assert_eq!(graph.schema_version, 1);
    assert_eq!(graph.repository, "BEACON OPS");
    assert_eq!(graph.nodes.len(), 29);
    assert_eq!(graph.edges.len(), 36);
    assert_eq!(graph.flows.len(), 1);
    assert_eq!(graph.scan_summary.source, "synthetic_fixture");
}

#[test]
fn every_edge_and_flow_reference_resolves() {
    let graph = load_beacon_ops().expect("fixture should load");
    let node_ids: HashSet<_> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let edges: HashMap<_, _> = graph
        .edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge))
        .collect();

    for edge in &graph.edges {
        assert!(
            node_ids.contains(edge.source.as_str()),
            "missing source for {}",
            edge.id
        );
        assert!(
            node_ids.contains(edge.target.as_str()),
            "missing target for {}",
            edge.id
        );
        assert!(!edge.evidence.path.is_empty());
        assert!(edge.evidence.line_start <= edge.evidence.line_end);
    }

    for flow in &graph.flows {
        assert_eq!(flow.node_ids.len(), flow.edge_ids.len() + 1);
        for (index, edge_id) in flow.edge_ids.iter().enumerate() {
            let edge = edges.get(edge_id.as_str()).expect("flow edge should exist");
            assert_eq!(edge.source, flow.node_ids[index]);
            assert_eq!(edge.target, flow.node_ids[index + 1]);
            assert_eq!(edge.provenance, Provenance::Extracted);
        }
    }
}

#[test]
fn uncertainty_is_sparse_and_explicit() {
    let graph = load_beacon_ops().expect("fixture should load");
    let inferred: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.provenance == Provenance::Inferred)
        .collect();

    assert_eq!(inferred.len(), 1);
    assert_eq!(inferred[0].target, "N27");
    assert!(inferred[0].confidence < 0.5);
    assert!(inferred[0].relationship.contains('?'));
}

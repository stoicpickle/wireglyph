use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub schema_version: u32,
    pub repository: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub flows: Vec<Flow>,
    pub scan_summary: ScanSummary,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub group: String,
    pub label: String,
    pub kind: NodeKind,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Entry,
    Module,
    Configuration,
    Component,
    Router,
    Route,
    Handler,
    Middleware,
    Service,
    Interface,
    Model,
    Adapter,
    ExternalPackage,
    ExternalSystem,
    ExternalService,
    Unresolved,
    Utility,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relationship: String,
    pub provenance: Provenance,
    pub confidence: f32,
    pub evidence: Evidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_specifier: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Extracted,
    Inferred,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub id: String,
    pub label: String,
    pub provenance: Provenance,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScanSummary {
    pub source: String,
    #[serde(default)]
    pub files_discovered: u32,
    pub files_scanned: u32,
    pub files_skipped: u32,
    #[serde(default)]
    pub skipped_by_reason: BTreeMap<String, u32>,
    #[serde(default)]
    pub parse_warnings: u32,
    #[serde(default)]
    pub traversal_errors: u32,
    pub inferred_edges: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanHealth {
    Complete,
    Partial,
}

impl ScanSummary {
    pub const fn health(&self) -> ScanHealth {
        if self.files_skipped > 0 || self.parse_warnings > 0 || self.traversal_errors > 0 {
            ScanHealth::Partial
        } else {
            ScanHealth::Complete
        }
    }

    pub const fn health_label(&self) -> &'static str {
        match self.health() {
            ScanHealth::Complete => "SCOPED OK",
            ScanHealth::Partial => "PARTIAL",
        }
    }
}

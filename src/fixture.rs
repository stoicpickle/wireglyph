use crate::model::Graph;

const BEACON_OPS: &str = include_str!("../fixtures/beacon_ops.json");

pub fn load_beacon_ops() -> Result<Graph, serde_json::Error> {
    serde_json::from_str(BEACON_OPS)
}

//! Embedded GraphQL operation catalog (reconstructed from the Acorns web app).
//! Single source of truth for both friendly commands and `raw`.

use serde::Deserialize;
use std::collections::BTreeMap;

const CATALOG_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/catalog.json"));

#[derive(Deserialize)]
pub struct Catalog {
    pub ops: BTreeMap<String, Op>,
    #[serde(rename = "inputTypes", default)]
    pub input_types: serde_json::Value,
}

#[derive(Deserialize, Clone)]
pub struct Op {
    pub kind: String,
    #[serde(default)]
    pub name: String,
    /// Executable GraphQL document (fragments inlined).
    pub doc: String,
    #[serde(default)]
    pub vars: Vec<Var>,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub roots: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub struct Var {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub required: bool,
}

impl Var {
    /// Base type with list/non-null decorations stripped (e.g. `[ID!]!` -> `ID`).
    pub fn base_type(&self) -> &str {
        self.ty.trim_matches(|c| c == '[' || c == ']' || c == '!')
    }
}

pub fn load() -> Catalog {
    serde_json::from_str(CATALOG_JSON).expect("embedded catalog.json must be valid")
}

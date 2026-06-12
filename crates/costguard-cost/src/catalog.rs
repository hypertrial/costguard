use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct CatalogStats {
    pub bytes_by_key: HashMap<String, u64>,
    pub rows_by_key: HashMap<String, u64>,
}

pub fn load_catalog(path: &Path) -> Result<CatalogStats> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read catalog {}", path.display()))?;
    parse_catalog_text(&text)
}

pub fn parse_catalog_text(text: &str) -> Result<CatalogStats> {
    let value: Value = serde_json::from_str(text).context("invalid catalog.json")?;
    let mut stats = CatalogStats::default();
    for section in ["nodes", "sources"] {
        if let Some(entries) = value.get(section).and_then(Value::as_object) {
            for (key, node) in entries {
                if let Some(num_bytes) = node.pointer("/stats/num_bytes").and_then(Value::as_u64) {
                    stats.bytes_by_key.insert(key.clone(), num_bytes);
                }
                if let Some(row_count) = node.pointer("/stats/row_count").and_then(Value::as_u64) {
                    stats.rows_by_key.insert(key.clone(), row_count);
                }
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_catalog_nodes() {
        let text = r#"{
            "nodes": {
                "model.project.fct_orders": {
                    "stats": {"num_bytes": 1000, "row_count": 10}
                }
            }
        }"#;
        let stats = parse_catalog_text(text).unwrap();
        assert_eq!(stats.bytes_by_key["model.project.fct_orders"], 1000);
    }
}

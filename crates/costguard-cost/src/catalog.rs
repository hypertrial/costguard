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
                if let Some(value) = node.pointer("/stats/num_bytes") {
                    let num_bytes = value.as_u64().with_context(|| {
                        format!("catalog num_bytes for {key} must be a positive integer")
                    })?;
                    if num_bytes == 0 {
                        anyhow::bail!("catalog num_bytes for {key} must be greater than zero");
                    }
                    stats.bytes_by_key.insert(key.clone(), num_bytes);
                }
                if let Some(value) = node.pointer("/stats/row_count") {
                    let row_count = value.as_u64().with_context(|| {
                        format!("catalog row_count for {key} must be a positive integer")
                    })?;
                    if row_count == 0 {
                        anyhow::bail!("catalog row_count for {key} must be greater than zero");
                    }
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

    #[test]
    fn rejects_zero_catalog_stats() {
        let text = r#"{"nodes":{"model.project.zero":{"stats":{"num_bytes":0}}}}"#;
        assert!(parse_catalog_text(text).is_err());
    }
}

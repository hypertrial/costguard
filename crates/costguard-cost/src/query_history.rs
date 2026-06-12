use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct QueryHistoryEntry {
    pub bytes_per_run: f64,
    pub runs_per_month: f64,
}

#[derive(Debug, Clone, Default)]
pub struct QueryHistoryStats {
    pub by_key: HashMap<String, QueryHistoryEntry>,
}

pub fn load_query_history(path: &Path) -> Result<QueryHistoryStats> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read query history {}", path.display()))?;
    parse_query_history_text(&text)
}

pub fn parse_query_history_text(text: &str) -> Result<QueryHistoryStats> {
    let mut stats = QueryHistoryStats::default();
    let mut lines = text.lines();
    let header = lines
        .next()
        .context("query history CSV is empty")?
        .trim()
        .to_ascii_lowercase();
    let columns: Vec<&str> = header.split(',').map(str::trim).collect();
    let model_idx = columns
        .iter()
        .position(|c| *c == "model_or_table" || *c == "model" || *c == "table")
        .context("query history CSV missing model_or_table column")?;
    let bytes_idx = columns
        .iter()
        .position(|c| *c == "bytes_per_run" || *c == "bytes_billed")
        .context("query history CSV missing bytes_per_run column")?;
    let runs_idx = columns
        .iter()
        .position(|c| *c == "runs_per_month" || *c == "run_count")
        .unwrap_or(usize::MAX);

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() <= model_idx.max(bytes_idx) {
            continue;
        }
        let key = fields[model_idx].to_string();
        let bytes: f64 = fields[bytes_idx]
            .parse()
            .with_context(|| format!("invalid bytes in line: {line}"))?;
        let runs = if runs_idx != usize::MAX && fields.len() > runs_idx {
            fields[runs_idx].parse().unwrap_or(30.0)
        } else {
            30.0
        };
        stats.by_key.insert(
            key,
            QueryHistoryEntry {
                bytes_per_run: bytes,
                runs_per_month: runs,
            },
        );
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query_history_csv() {
        let text = "model_or_table,bytes_per_run,runs_per_month\nfct_orders,1e12,720\n";
        let stats = parse_query_history_text(text).unwrap();
        assert_eq!(stats.by_key["fct_orders"].bytes_per_run, 1e12);
        assert_eq!(stats.by_key["fct_orders"].runs_per_month, 720.0);
    }
}

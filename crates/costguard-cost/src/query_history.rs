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
    let records = crate::csv::csv_records(text)?;
    let (header, body) = records
        .split_first()
        .context("query history CSV is empty")?;
    let columns: Vec<String> = header
        .iter()
        .map(|field| field.trim().to_ascii_lowercase())
        .collect();
    if columns.iter().any(|column| column.is_empty()) {
        anyhow::bail!("query history CSV headers cannot be empty");
    }
    let model_idx = columns
        .iter()
        .position(|c| c == "model_or_table" || c == "model" || c == "table")
        .context("query history CSV missing model_or_table column")?;
    let bytes_idx = columns
        .iter()
        .position(|c| c == "bytes_per_run" || c == "bytes_billed")
        .context("query history CSV missing bytes_per_run column")?;
    let runs_idx = columns
        .iter()
        .position(|c| c == "runs_per_month" || c == "run_count");

    let mut stats = QueryHistoryStats::default();
    for (index, row) in body.iter().enumerate() {
        if row.len() != header.len() {
            anyhow::bail!(
                "query history CSV row {} has {} fields; expected {}",
                index + 2,
                row.len(),
                header.len()
            );
        }
        let key = row[model_idx].trim();
        if key.is_empty() {
            anyhow::bail!(
                "query history CSV row {} has empty model_or_table",
                index + 2
            );
        }
        if stats.by_key.contains_key(key) {
            anyhow::bail!(
                "query history CSV row {} duplicates model_or_table '{key}'",
                index + 2
            );
        }
        let bytes: f64 = row[bytes_idx]
            .trim()
            .parse()
            .with_context(|| format!("invalid bytes in row {}", index + 2))?;
        validate_positive("bytes_per_run", bytes, index + 2)?;
        let runs = if let Some(idx) = runs_idx {
            let parsed = row[idx]
                .trim()
                .parse::<f64>()
                .with_context(|| format!("invalid runs_per_month in row {}", index + 2))?;
            validate_positive("runs_per_month", parsed, index + 2)?;
            parsed
        } else {
            30.0
        };
        stats.by_key.insert(
            key.to_string(),
            QueryHistoryEntry {
                bytes_per_run: bytes,
                runs_per_month: runs,
            },
        );
    }
    Ok(stats)
}

fn validate_positive(name: &str, value: f64, row: usize) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        anyhow::bail!("{name} must be finite and greater than zero in row {row}");
    }
    Ok(())
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

    #[test]
    fn parses_quoted_history_row() {
        let text = "model_or_table,bytes_per_run,runs_per_month\n\"fct,orders\",500,60\n";
        let stats = parse_query_history_text(text).unwrap();
        assert_eq!(stats.by_key["fct,orders"].bytes_per_run, 500.0);
    }

    #[test]
    fn parses_multiline_quoted_history_row() {
        let text = "model_or_table,bytes_per_run,runs_per_month\n\"fct\norders\",500,60\n";
        let stats = parse_query_history_text(text).unwrap();
        assert_eq!(stats.by_key["fct\norders"].bytes_per_run, 500.0);
    }

    #[test]
    fn rejects_non_positive_history_values() {
        let text = "model_or_table,bytes_per_run,runs_per_month\nfct_orders,1000,0\n";
        assert!(parse_query_history_text(text).is_err());
    }

    #[test]
    fn rejects_truncated_history_rows() {
        let text = "model_or_table,bytes_per_run,runs_per_month\nfct_orders,1000\n";
        assert!(parse_query_history_text(text).is_err());
    }

    #[test]
    fn rejects_empty_model_names() {
        let text = "model_or_table,bytes_per_run,runs_per_month\n,1000,30\n";
        assert!(parse_query_history_text(text).is_err());
    }

    #[test]
    fn rejects_duplicate_model_names() {
        let text =
            "model_or_table,bytes_per_run,runs_per_month\nfct_orders,1000,30\nfct_orders,500,60\n";
        assert!(parse_query_history_text(text).is_err());
    }
}

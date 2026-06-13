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
    let columns: Vec<String> = parse_csv_line(&header)
        .into_iter()
        .map(|field| field.trim().to_ascii_lowercase())
        .collect();
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

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_line(line);
        if fields.len() <= model_idx.max(bytes_idx) {
            continue;
        }
        let key = fields[model_idx].trim().to_string();
        let bytes: f64 = fields[bytes_idx]
            .trim()
            .parse()
            .with_context(|| format!("invalid bytes in line: {line}"))?;
        validate_positive("bytes_per_run", bytes, line)?;
        let runs = if let Some(idx) = runs_idx {
            let value = fields
                .get(idx)
                .context("query history row is missing runs_per_month")?;
            let parsed = value
                .trim()
                .parse::<f64>()
                .with_context(|| format!("invalid runs_per_month in line: {line}"))?;
            validate_positive("runs_per_month", parsed, line)?;
            parsed
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

fn validate_positive(name: &str, value: f64, line: &str) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        anyhow::bail!("{name} must be finite and greater than zero in line: {line}");
    }
    Ok(())
}

/// Minimal RFC4180-style CSV line parser (handles quoted fields with commas).
pub fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    fields.push(current);
    fields
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
    fn parses_quoted_csv_fields() {
        let line = r#""model,with,comma",1000,30"#;
        let fields = parse_csv_line(line);
        assert_eq!(fields[0], "model,with,comma");
        assert_eq!(fields[1], "1000");
    }

    #[test]
    fn parses_quoted_history_row() {
        let text = "model_or_table,bytes_per_run,runs_per_month\n\"fct,orders\",500,60\n";
        let stats = parse_query_history_text(text).unwrap();
        assert_eq!(stats.by_key["fct,orders"].bytes_per_run, 500.0);
    }

    #[test]
    fn rejects_non_positive_history_values() {
        let text = "model_or_table,bytes_per_run,runs_per_month\nfct_orders,1000,0\n";
        assert!(parse_query_history_text(text).is_err());
    }
}

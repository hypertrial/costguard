use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use costguard_protocol::{CostObservationBundleV1, ModelCostObservationV1};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostExportFormat {
    Normalized,
    Snowflake,
    BigQuery,
    Databricks,
    Trino,
    Pipeline,
    Generic,
}

impl FromStr for CostExportFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "normalized" => Ok(Self::Normalized),
            "snowflake" => Ok(Self::Snowflake),
            "bigquery" => Ok(Self::BigQuery),
            "databricks" => Ok(Self::Databricks),
            "trino" => Ok(Self::Trino),
            "pipeline" => Ok(Self::Pipeline),
            "generic" => Ok(Self::Generic),
            other => Err(format!("unknown cost export format '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NormalizeCostOptions {
    pub organization: String,
    pub repository: String,
    pub currency: String,
    pub provenance: String,
    pub model_mapping: BTreeMap<String, String>,
}

pub fn normalize_cost_export(
    input: &str,
    format: CostExportFormat,
    options: &NormalizeCostOptions,
) -> Result<CostObservationBundleV1> {
    if format == CostExportFormat::Normalized {
        let bundle: CostObservationBundleV1 =
            serde_json::from_str(input).context("invalid normalized cost bundle JSON")?;
        validate_cost_bundle(&bundle)?;
        return Ok(bundle);
    }
    let rows = parse_rows(input)?;
    let observations = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            normalize_row(row, format, options).with_context(|| format!("cost row {}", index + 1))
        })
        .collect::<Result<Vec<_>>>()?;
    let bundle = CostObservationBundleV1 {
        schema_version: 1,
        organization: options.organization.clone(),
        repository: options.repository.clone(),
        currency: options.currency.to_ascii_uppercase(),
        generated_at: Utc::now().to_rfc3339(),
        provenance: options.provenance.clone(),
        observations,
    };
    validate_cost_bundle(&bundle)?;
    Ok(bundle)
}

pub fn validate_cost_bundle(bundle: &CostObservationBundleV1) -> Result<()> {
    if bundle.schema_version != 1 {
        anyhow::bail!("unsupported cost bundle schema {}", bundle.schema_version);
    }
    if bundle.organization.trim().is_empty()
        || bundle.repository.trim().is_empty()
        || bundle.provenance.trim().is_empty()
    {
        anyhow::bail!("organization, repository, and provenance are required");
    }
    if bundle.currency != "USD" {
        anyhow::bail!("v2 cost reporting supports normalized USD only");
    }
    DateTime::parse_from_rfc3339(&bundle.generated_at).context("generated_at must be RFC3339")?;
    let mut unique = BTreeSet::new();
    for observation in &bundle.observations {
        if observation.model_id.trim().is_empty() {
            anyhow::bail!("cost observation model_id cannot be empty");
        }
        let start = parse_timestamp(&observation.window_start)?;
        let end = parse_timestamp(&observation.window_end)?;
        if end <= start {
            anyhow::bail!("cost observation window_end must be after window_start");
        }
        for value in [
            observation.bytes_processed,
            observation.compute_seconds,
            observation.credits,
            observation.cost_usd,
        ]
        .into_iter()
        .flatten()
        {
            if !value.is_finite() || value < 0.0 {
                anyhow::bail!("cost observation values must be finite and non-negative");
            }
        }
        if !unique.insert((
            observation.model_id.as_str(),
            observation.window_start.as_str(),
            observation.window_end.as_str(),
        )) {
            anyhow::bail!("duplicate model/time-window cost observation");
        }
    }
    Ok(())
}

fn normalize_row(
    row: &Value,
    format: CostExportFormat,
    options: &NormalizeCostOptions,
) -> Result<ModelCostObservationV1> {
    let aliases = aliases(format);
    let source_model = string_value(row, aliases.model).context("missing model identifier")?;
    let model_id = options
        .model_mapping
        .get(&source_model)
        .cloned()
        .unwrap_or(source_model);
    let start =
        parse_timestamp(&string_value(row, aliases.start).context("missing window start")?)?;
    let end = parse_timestamp(&string_value(row, aliases.end).context("missing window end")?)?;
    let executions = number_value(row, aliases.executions).unwrap_or(1.0);
    if !executions.is_finite() || executions < 0.0 || executions.fract() != 0.0 {
        anyhow::bail!("executions must be a non-negative integer");
    }
    let mut compute_seconds = number_value(row, aliases.compute_seconds);
    if compute_seconds.is_none() {
        compute_seconds =
            number_value(row, aliases.compute_milliseconds).map(|value| value / 1_000.0);
    }
    Ok(ModelCostObservationV1 {
        model_id,
        window_start: start.to_rfc3339(),
        window_end: end.to_rfc3339(),
        executions: executions as u64,
        bytes_processed: number_value(row, aliases.bytes),
        compute_seconds,
        credits: number_value(row, aliases.credits),
        cost_usd: number_value(row, aliases.cost_usd),
    })
}

struct Aliases {
    model: &'static [&'static str],
    start: &'static [&'static str],
    end: &'static [&'static str],
    executions: &'static [&'static str],
    bytes: &'static [&'static str],
    compute_seconds: &'static [&'static str],
    compute_milliseconds: &'static [&'static str],
    credits: &'static [&'static str],
    cost_usd: &'static [&'static str],
}

fn aliases(format: CostExportFormat) -> Aliases {
    let common = Aliases {
        model: &[
            "model_id",
            "model",
            "query_tag",
            "job_name",
            "resource_name",
        ],
        start: &["window_start", "start_time", "start", "usage_start_time"],
        end: &["window_end", "end_time", "end", "usage_end_time"],
        executions: &["executions", "execution_count", "query_count", "count"],
        bytes: &[
            "bytes_processed",
            "bytes_scanned",
            "bytes_billed",
            "total_bytes_billed",
        ],
        compute_seconds: &[
            "compute_seconds",
            "execution_seconds",
            "total_cpu_time_seconds",
        ],
        compute_milliseconds: &["execution_time_ms", "total_slot_ms", "wall_time_ms"],
        credits: &[
            "credits",
            "credits_used",
            "credits_used_compute",
            "usage_quantity",
        ],
        cost_usd: &["cost_usd", "usd_cost", "cost", "list_cost"],
    };
    match format {
        CostExportFormat::Snowflake => Aliases {
            model: &["query_tag", "model_id", "model"],
            ..common
        },
        CostExportFormat::BigQuery => Aliases {
            model: &["labels.model_id", "model_id", "job_id"],
            ..common
        },
        CostExportFormat::Databricks => Aliases {
            model: &["usage_metadata.job_name", "job_name", "model_id"],
            ..common
        },
        CostExportFormat::Trino => Aliases {
            model: &["model_id", "query_id", "source"],
            ..common
        },
        CostExportFormat::Pipeline => Aliases {
            model: &[
                "model_id",
                "model",
                "relation",
                "relation_name",
                "asset_key",
                "asset",
                "asset_name",
                "dbt_model",
                "dbt_model_id",
                "resource_name",
            ],
            executions: &[
                "executions",
                "run_count",
                "materialization_count",
                "execution_count",
                "count",
            ],
            compute_seconds: &[
                "compute_seconds",
                "duration_seconds",
                "elapsed_seconds",
                "execution_seconds",
                "total_cpu_time_seconds",
            ],
            compute_milliseconds: &[
                "duration_ms",
                "duration_milliseconds",
                "elapsed_ms",
                "execution_time_ms",
                "wall_time_ms",
            ],
            ..common
        },
        CostExportFormat::Generic | CostExportFormat::Normalized => common,
    }
}

fn parse_rows(input: &str) -> Result<Vec<Value>> {
    let trimmed = input.trim_start();
    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).context("invalid cost export JSON array");
    }
    if trimmed.starts_with('{') {
        let value: Value = serde_json::from_str(trimmed).context("invalid cost export JSON")?;
        if let Some(rows) = value.get("rows").and_then(Value::as_array) {
            return Ok(rows.clone());
        }
        return Ok(vec![value]);
    }
    parse_csv(input)
}

fn parse_csv(input: &str) -> Result<Vec<Value>> {
    let records = crate::csv::csv_records(input)?;
    let (header, body) = records.split_first().context("CSV export is empty")?;
    if header.iter().any(|field| field.trim().is_empty()) {
        anyhow::bail!("CSV headers cannot be empty");
    }
    body.iter()
        .enumerate()
        .map(|(index, row)| {
            if row.len() != header.len() {
                anyhow::bail!(
                    "CSV row {} has {} fields; expected {}",
                    index + 2,
                    row.len(),
                    header.len()
                );
            }
            Ok(Value::Object(
                header
                    .iter()
                    .cloned()
                    .zip(row.iter().cloned().map(Value::String))
                    .collect(),
            ))
        })
        .collect()
}

fn string_value(row: &Value, aliases: &[&str]) -> Option<String> {
    aliases
        .iter()
        .find_map(|alias| nested_value(row, alias))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .filter(|value| !value.trim().is_empty())
}

fn number_value(row: &Value, aliases: &[&str]) -> Option<f64> {
    aliases
        .iter()
        .find_map(|alias| nested_value(row, alias))
        .and_then(|value| match value {
            Value::Number(value) => value.as_f64(),
            Value::String(value) => value.trim().parse().ok(),
            _ => None,
        })
}

fn nested_value<'a>(row: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(row, |value, key| value.get(key))
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(parsed.and_utc());
        }
    }
    anyhow::bail!("timestamp '{value}' must be RFC3339 or an unzoned UTC timestamp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> NormalizeCostOptions {
        NormalizeCostOptions {
            organization: "acme".into(),
            repository: "acme/warehouse".into(),
            currency: "USD".into(),
            provenance: "snowflake-export-2026-06".into(),
            model_mapping: BTreeMap::from([("orders".into(), "model.acme.orders".into())]),
        }
    }

    #[test]
    fn normalizes_snowflake_csv_without_source_content() {
        let input = "query_tag,start_time,end_time,query_count,bytes_scanned,credits_used_compute,cost_usd\norders,2026-06-01T00:00:00Z,2026-06-02T00:00:00Z,4,1000,1.5,4.25\n";
        let bundle = normalize_cost_export(input, CostExportFormat::Snowflake, &options()).unwrap();
        assert_eq!(bundle.observations[0].model_id, "model.acme.orders");
        assert_eq!(bundle.observations[0].executions, 4);
        assert_eq!(bundle.observations[0].credits, Some(1.5));
        let value = serde_json::to_string(&bundle).unwrap();
        assert!(!value.contains("query_text"));
    }

    #[test]
    fn normalizes_pipeline_csv_aliases_and_ignores_rows() {
        let input = "relation,window_start,window_end,run_count,bytes_processed,duration_ms,rows_written,usd_cost\norders,2026-06-01T00:00:00Z,2026-06-02T00:00:00Z,2,2048,1500,99,3.50\n";
        let bundle = normalize_cost_export(input, CostExportFormat::Pipeline, &options()).unwrap();
        let observation = &bundle.observations[0];
        assert_eq!(observation.model_id, "model.acme.orders");
        assert_eq!(observation.executions, 2);
        assert_eq!(observation.bytes_processed, Some(2048.0));
        assert_eq!(observation.compute_seconds, Some(1.5));
        assert_eq!(observation.cost_usd, Some(3.5));
    }

    #[test]
    fn normalizes_pipeline_json_rows_with_default_execution_count() {
        let input = r#"{"rows":[{"asset_key":"model.acme.events","window_start":"2026-06-01 00:00:00","window_end":"2026-06-02 00:00:00","duration_seconds":7}]}"#;
        let bundle = normalize_cost_export(input, CostExportFormat::Pipeline, &options()).unwrap();
        let observation = &bundle.observations[0];
        assert_eq!(observation.model_id, "model.acme.events");
        assert_eq!(observation.executions, 1);
        assert_eq!(observation.compute_seconds, Some(7.0));
    }

    #[test]
    fn normalizes_pipeline_json_array_and_object_inputs() {
        let array = r#"[{"dbt_model":"orders","window_start":"2026-06-01T00:00:00Z","window_end":"2026-06-02T00:00:00Z","duration_seconds":4}]"#;
        let bundle = normalize_cost_export(array, CostExportFormat::Pipeline, &options()).unwrap();
        assert_eq!(bundle.observations[0].model_id, "model.acme.orders");
        assert_eq!(bundle.observations[0].compute_seconds, Some(4.0));

        let object = r#"{"relation_name":"model.acme.order_items","window_start":"2026-06-01T00:00:00Z","window_end":"2026-06-02T00:00:00Z","run_count":3}"#;
        let bundle = normalize_cost_export(object, CostExportFormat::Pipeline, &options()).unwrap();
        assert_eq!(bundle.observations[0].model_id, "model.acme.order_items");
        assert_eq!(bundle.observations[0].executions, 3);
    }

    #[test]
    fn rejects_pipeline_duplicates_and_negative_values() {
        let negative = normalize_cost_export(
            "relation,window_start,window_end,cost_usd\norders,2026-01-01T00:00:00Z,2026-01-02T00:00:00Z,-1\n",
            CostExportFormat::Pipeline,
            &options(),
        );
        assert!(negative.is_err());
        let duplicate = normalize_cost_export(
            "relation,window_start,window_end\norders,2026-01-01T00:00:00Z,2026-01-02T00:00:00Z\norders,2026-01-01T00:00:00Z,2026-01-02T00:00:00Z\n",
            CostExportFormat::Pipeline,
            &options(),
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn rejects_duplicates_and_negative_values() {
        let mut bundle = normalize_cost_export(
            "model_id,window_start,window_end,cost_usd\na,2026-01-01T00:00:00Z,2026-01-02T00:00:00Z,-1\n",
            CostExportFormat::Generic,
            &options(),
        );
        assert!(bundle.is_err());
        bundle = normalize_cost_export(
            "model_id,window_start,window_end\na,2026-01-01T00:00:00Z,2026-01-02T00:00:00Z\na,2026-01-01T00:00:00Z,2026-01-02T00:00:00Z\n",
            CostExportFormat::Generic,
            &options(),
        );
        assert!(bundle.is_err());
    }
}

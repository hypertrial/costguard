use crate::config::CostConfig;
use crate::import::validate_cost_bundle;
use crate::pricing::price_per_byte;
use crate::Estimate;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use costguard_protocol::{CostObservationBundleV1, ModelCostObservationV1};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ObservedModelCost {
    pub monthly_cost_usd: Option<Estimate>,
    pub monthly_credits: Option<f64>,
    pub monthly_bytes: Option<f64>,
    pub monthly_executions: f64,
    pub window_start: String,
    pub window_end: String,
    pub age_days: f64,
    pub basis: String,
}

#[derive(Debug, Clone, Default)]
pub struct ObservationStats {
    pub by_model: HashMap<String, ObservedModelCost>,
    pub generated_at: Option<String>,
    pub provenance: Option<String>,
}

pub fn load_observations(path: &Path) -> Result<ObservationStats> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read observations {}", path.display()))?;
    parse_observations_text(&text)
}

pub fn parse_observations_text(text: &str) -> Result<ObservationStats> {
    let bundle: CostObservationBundleV1 =
        serde_json::from_str(text).context("invalid observations bundle JSON")?;
    validate_cost_bundle(&bundle)?;
    Ok(aggregate_bundle(&bundle, &CostConfig::default()))
}

pub fn aggregate_bundle(bundle: &CostObservationBundleV1, config: &CostConfig) -> ObservationStats {
    let price = price_per_byte(config);
    let usd_per_credit = config.pricing.usd_per_credit.unwrap_or(3.0);
    let mut by_model: HashMap<String, Vec<&ModelCostObservationV1>> = HashMap::new();
    for obs in &bundle.observations {
        by_model.entry(obs.model_id.clone()).or_default().push(obs);
    }
    let now = Utc::now();
    let mut stats = ObservationStats {
        generated_at: Some(bundle.generated_at.clone()),
        provenance: Some(bundle.provenance.clone()),
        ..Default::default()
    };
    for (model_id, observations) in by_model {
        if let Some(aggregated) =
            aggregate_model_observations(&observations, price, usd_per_credit, now)
        {
            stats.by_model.insert(model_id, aggregated);
        }
    }
    stats
}

fn aggregate_model_observations(
    observations: &[&ModelCostObservationV1],
    price: Option<Estimate>,
    usd_per_credit: f64,
    now: DateTime<Utc>,
) -> Option<ObservedModelCost> {
    if observations.is_empty() {
        return None;
    }
    let mut window_start: Option<DateTime<Utc>> = None;
    let mut window_end: Option<DateTime<Utc>> = None;
    let mut total_usd = 0.0_f64;
    let mut total_credits = 0.0_f64;
    let mut total_bytes = 0.0_f64;
    let mut total_executions = 0.0_f64;
    let mut has_usd = false;
    let mut has_credits = false;
    let mut has_bytes = false;

    for obs in observations {
        let start = parse_ts(&obs.window_start).ok()?;
        let end = parse_ts(&obs.window_end).ok()?;
        window_start = Some(window_start.map_or(start, |ws| ws.min(start)));
        window_end = Some(window_end.map_or(end, |we| we.max(end)));
        total_executions += obs.executions as f64;
        if let Some(usd) = obs.cost_usd {
            total_usd += usd;
            has_usd = true;
        }
        if let Some(credits) = obs.credits {
            total_credits += credits;
            has_credits = true;
        }
        if let Some(bytes) = obs.bytes_processed {
            total_bytes += bytes;
            has_bytes = true;
        }
    }

    let start = window_start?;
    let end = window_end?;
    let span_days = (end - start).num_seconds() as f64 / 86_400.0;
    let months = (span_days / 30.0).max(1.0 / 30.0);
    let age_days = (now - end).num_seconds() as f64 / 86_400.0;

    let (monthly_cost_usd, basis) = if has_usd {
        (
            Some(Estimate::from_point(total_usd / months, Some(0.1))),
            "observed-usd".to_string(),
        )
    } else if has_credits {
        (
            Some(Estimate::from_point(
                (total_credits / months) * usd_per_credit,
                Some(0.15),
            )),
            "observed-credits".to_string(),
        )
    } else if has_bytes {
        if let Some(price) = price {
            let monthly_bytes = total_bytes / months;
            (
                Some(Estimate::from_point(monthly_bytes, Some(0.15)) * price),
                "observed-bytes".to_string(),
            )
        } else {
            (
                Some(Estimate::from_point(total_bytes / months, Some(0.15))),
                "observed-bytes".to_string(),
            )
        }
    } else {
        return None;
    };

    Some(ObservedModelCost {
        monthly_cost_usd,
        monthly_credits: has_credits.then(|| total_credits / months),
        monthly_bytes: has_bytes.then(|| total_bytes / months),
        monthly_executions: total_executions / months,
        window_start: start.to_rfc3339(),
        window_end: end.to_rfc3339(),
        age_days: age_days.max(0.0),
        basis,
    })
}

fn parse_ts(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|ts| ts.with_timezone(&Utc))
        .context("invalid observation timestamp")
}

pub fn lookup_observed_cost<'a>(
    stats: &'a ObservationStats,
    keys: &[String],
) -> Option<&'a ObservedModelCost> {
    for key in keys {
        if let Some(cost) = stats.by_model.get(key) {
            return Some(cost);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_observations_to_monthly_rate() {
        let text = r#"{
            "schema_version": 1,
            "organization": "acme",
            "repository": "acme/warehouse",
            "currency": "USD",
            "generated_at": "2026-06-15T00:00:00Z",
            "provenance": "test",
            "observations": [
                {
                    "model_id": "model.acme.orders",
                    "window_start": "2026-06-01T00:00:00Z",
                    "window_end": "2026-06-02T00:00:00Z",
                    "executions": 4,
                    "cost_usd": 8.5
                },
                {
                    "model_id": "model.acme.orders",
                    "window_start": "2026-06-02T00:00:00Z",
                    "window_end": "2026-06-03T00:00:00Z",
                    "executions": 6,
                    "cost_usd": 11.5
                }
            ]
        }"#;
        let stats = parse_observations_text(text).unwrap();
        let cost = &stats.by_model["model.acme.orders"];
        assert_eq!(cost.basis, "observed-usd");
        assert!((cost.monthly_cost_usd.as_ref().unwrap().median() - 300.0).abs() < 1.0);
        assert!((cost.monthly_executions - 150.0).abs() < 1.0);
    }

    #[test]
    fn prefers_usd_over_credits() {
        let text = r#"{
            "schema_version": 1,
            "organization": "acme",
            "repository": "acme/warehouse",
            "currency": "USD",
            "generated_at": "2026-06-15T00:00:00Z",
            "provenance": "test",
            "observations": [{
                "model_id": "m",
                "window_start": "2026-06-01T00:00:00Z",
                "window_end": "2026-07-01T00:00:00Z",
                "executions": 1,
                "cost_usd": 30.0,
                "credits": 100.0
            }]
        }"#;
        let stats = parse_observations_text(text).unwrap();
        assert_eq!(stats.by_model["m"].basis, "observed-usd");
    }
}

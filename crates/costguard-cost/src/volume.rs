use crate::catalog::CatalogStats;
use crate::config::{parse_bytes_spec, CostConfig, CostSourceOverride, Warehouse};
use crate::observations::{lookup_observed_cost, ObservationStats};
use crate::query_history::QueryHistoryStats;
use crate::Estimate;
use costguard_dbt::{DbtModel, DbtProject};
use costguard_diagnostics::CostGrade;

pub const DEFAULT_AVG_ROW_BYTES: f64 = 200.0;

/// Rules whose findings indicate incremental bounds are missing — do not discount bytes.
pub fn skips_incremental_discount(rule_id: &str) -> bool {
    matches!(
        rule_id.to_ascii_uppercase().as_str(),
        "SQLCOST004" | "SQLCOST005" | "SQLCOST019" | "SQLCOST029"
    )
}

pub struct ResolvedVolume {
    pub bytes: Estimate,
    pub runs_per_month: Estimate,
    pub grade: CostGrade,
    #[allow(dead_code)] // read in tests; production skips priors via separate Grade-A code paths
    pub measured: bool,
    pub estimation_basis: String,
    pub observation_window: Option<(String, String)>,
    pub observation_age_days: Option<f64>,
    pub observed_monthly_cost: Option<Estimate>,
}

pub struct VolumeContext<'a> {
    pub config: &'a CostConfig,
    pub observations: Option<&'a ObservationStats>,
    pub catalog: Option<&'a CatalogStats>,
    pub query_history: Option<&'a QueryHistoryStats>,
    pub dbt: Option<&'a DbtProject>,
}

impl VolumeContext<'_> {
    pub fn resolve_for_model(&self, model: &DbtModel) -> ResolvedVolume {
        self.resolve_for_model_with_rule(model, None)
    }

    pub fn resolve_for_model_with_rule(
        &self,
        model: &DbtModel,
        rule_id: Option<&str>,
    ) -> ResolvedVolume {
        let apply_incremental = rule_id.is_none_or(|rule| !skips_incremental_discount(rule));
        let runs = self.runs_for_model(model);

        if let Some(stats) = self.observations {
            if let Some(observed) = lookup_observed_cost(stats, &lookup_keys(model)) {
                let monthly_cost = observed.monthly_cost_usd.unwrap_or_else(|| {
                    Estimate::from_point(observed.monthly_bytes.unwrap_or(0.0), Some(0.1))
                });
                let runs_per_month =
                    Estimate::from_point(observed.monthly_executions.max(1.0), Some(0.1));
                let bytes = if let Some(monthly_bytes) = observed.monthly_bytes {
                    if observed.monthly_executions > 0.0 {
                        Estimate::from_point(monthly_bytes / observed.monthly_executions, Some(0.1))
                    } else {
                        Estimate::from_point(monthly_bytes, Some(0.1))
                    }
                } else {
                    monthly_cost / runs_per_month
                };
                return ResolvedVolume {
                    bytes,
                    runs_per_month,
                    grade: CostGrade::A,
                    measured: true,
                    estimation_basis: observed.basis.clone(),
                    observation_window: Some((
                        observed.window_start.clone(),
                        observed.window_end.clone(),
                    )),
                    observation_age_days: Some(observed.age_days),
                    observed_monthly_cost: Some(monthly_cost),
                };
            }
        }

        if let Some(history) = self.query_history {
            for key in lookup_keys(model) {
                if let Some(entry) = history.by_key.get(&key) {
                    let bytes = Estimate::from_point(entry.bytes_per_run, Some(0.2));
                    return resolved_volume(
                        bytes,
                        Estimate::from_point(entry.runs_per_month, Some(0.1)),
                        CostGrade::A,
                        true,
                        "query-history",
                    );
                }
            }
        }

        if let Some(catalog) = self.catalog {
            if let Some(bytes) = catalog_bytes_for_model(catalog, model) {
                let mut est = bytes;
                est = apply_dbt_scan_priors(model, est, self.config.warehouse);
                if apply_incremental {
                    est = self.apply_incremental(model, est);
                }
                return resolved_volume(est, runs, CostGrade::B, false, "catalog");
            }
        }

        if let Some(source) = self.resolve_config_source(model, apply_incremental) {
            return source;
        }

        let prior_bytes = self.config.default_table_size.default_bytes();
        let mut est = Estimate::from_point(prior_bytes, Some(0.8));
        est = apply_dbt_scan_priors(model, est, self.config.warehouse);
        if apply_incremental {
            est = self.apply_incremental(model, est);
        }
        resolved_volume(est, runs, CostGrade::C, false, "size-prior")
    }

    fn runs_for_model(&self, model: &DbtModel) -> Estimate {
        for key in lookup_keys(model) {
            if let Some(override_cfg) = self.config.models.get(&key) {
                if let Some(runs) = override_cfg.runs_per_month {
                    return Estimate::from_point(runs, Some(0.1));
                }
            }
        }
        if let Some(inferred) = self.infer_runs_from_siblings(model) {
            return inferred;
        }
        Estimate::from_point(self.config.default_runs_per_month, Some(0.2))
    }

    fn infer_runs_from_siblings(&self, model: &DbtModel) -> Option<Estimate> {
        let history = self.query_history?;
        let folder = model.path.as_ref()?.parent()?.to_string_lossy().to_string();
        let dbt = self.dbt?;
        let mut runs: Vec<f64> = Vec::new();
        for other in dbt.models.values() {
            let Some(other_folder) = other
                .path
                .as_ref()
                .and_then(|path| path.parent())
                .map(|parent| parent.to_string_lossy().to_string())
            else {
                continue;
            };
            if other_folder != folder {
                continue;
            }
            for key in lookup_keys(other) {
                if let Some(entry) = history.by_key.get(&key) {
                    runs.push(entry.runs_per_month);
                    break;
                }
            }
        }
        if runs.is_empty() {
            return None;
        }
        runs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = runs[runs.len() / 2];
        Some(Estimate::from_point(median, Some(0.15)))
    }

    fn apply_incremental(&self, model: &DbtModel, bytes: Estimate) -> Estimate {
        if should_apply_incremental_discount(model) {
            scale_estimate(bytes, self.config.incremental_fraction)
        } else {
            bytes
        }
    }

    fn resolve_config_source(
        &self,
        model: &DbtModel,
        apply_incremental: bool,
    ) -> Option<ResolvedVolume> {
        let keys = upstream_keys(model, self.dbt);
        for key in keys {
            if let Some(source) = self.config.sources.get(&key) {
                if let Some(bytes) = source_bytes_estimate(source) {
                    let runs = self.runs_for_model(model);
                    let mut est = apply_dbt_scan_priors(model, bytes, self.config.warehouse);
                    if apply_incremental {
                        est = self.apply_incremental(model, est);
                    }
                    return Some(resolved_volume(
                        est,
                        runs,
                        CostGrade::B,
                        false,
                        "config-source",
                    ));
                }
            }
        }
        None
    }
}

fn resolved_volume(
    bytes: Estimate,
    runs_per_month: Estimate,
    grade: CostGrade,
    measured: bool,
    basis: &str,
) -> ResolvedVolume {
    ResolvedVolume {
        bytes,
        runs_per_month,
        grade,
        measured,
        estimation_basis: basis.to_string(),
        observation_window: None,
        observation_age_days: None,
        observed_monthly_cost: None,
    }
}

pub fn scale_estimate(bytes: Estimate, factor: f64) -> Estimate {
    Estimate {
        mu: bytes.mu + factor.ln(),
        sigma: bytes.sigma,
    }
}

fn should_apply_incremental_discount(model: &DbtModel) -> bool {
    if model.materialized.as_deref() != Some("incremental") {
        return false;
    }
    if model.full_refresh == Some(true) {
        return false;
    }
    model.unique_key.is_some()
}

fn apply_dbt_scan_priors(model: &DbtModel, bytes: Estimate, warehouse: Warehouse) -> Estimate {
    let mut factor = 1.0_f64;
    match warehouse {
        Warehouse::BigQuery => {
            if model.partition_by.is_some() {
                factor *= 0.5;
            } else if model.cluster_by.is_some() {
                factor *= 0.7;
            }
        }
        Warehouse::Snowflake => {
            if model.cluster_by.is_some() {
                factor *= 0.8;
            } else if model.partition_by.is_some() {
                factor *= 0.75;
            }
        }
        Warehouse::Databricks => {
            if model.partition_by.is_some() || model.cluster_by.is_some() {
                factor *= 0.75;
            }
        }
        _ => {
            if model.partition_by.is_some() || model.cluster_by.is_some() {
                factor *= 0.7;
            }
        }
    }
    if model.materialized.as_deref() == Some("view") {
        factor *= 0.5;
    }
    if factor == 1.0 {
        bytes
    } else {
        scale_estimate(bytes, factor)
    }
}

fn catalog_bytes_for_model(catalog: &CatalogStats, model: &DbtModel) -> Option<Estimate> {
    for key in lookup_keys(model) {
        if let Some(bytes) = catalog.bytes_by_key.get(&key) {
            return Some(Estimate::from_point(*bytes as f64, Some(0.25)));
        }
        if let Some(rows) = catalog.rows_by_key.get(&key) {
            let bytes = *rows as f64 * DEFAULT_AVG_ROW_BYTES;
            if bytes > 0.0 {
                return Some(Estimate::from_point(bytes, Some(0.35)));
            }
        }
    }
    None
}

fn source_bytes_estimate(source: &CostSourceOverride) -> Option<Estimate> {
    if let Some(bytes) = &source.bytes {
        let value = parse_bytes_spec(bytes).ok()?;
        return Some(Estimate::from_point(value, Some(0.3)));
    }
    if let Some(rows) = &source.rows {
        let row_est = super::config::range_or_point_to_estimate(rows, Some(0.3));
        let avg = source.avg_row_bytes.unwrap_or(DEFAULT_AVG_ROW_BYTES);
        return Some(Estimate::from_point(row_est.median() * avg, Some(0.4)));
    }
    None
}

pub fn lookup_keys(model: &DbtModel) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(id) = &model.unique_id {
        keys.push(id.clone());
    }
    if let Some(node_id) = &model.node_id {
        keys.push(node_id.clone());
    }
    if !model.fqn.is_empty() {
        keys.push(model.fqn.join("."));
    }
    keys.push(model.name.clone());
    keys
}

fn upstream_keys(model: &DbtModel, dbt: Option<&DbtProject>) -> Vec<String> {
    let mut keys = Vec::new();
    for source in &model.sources {
        keys.push(format!("{}.{}", source.source_name, source.table_name));
        keys.push(source.table_name.clone());
    }
    for reference in &model.refs {
        keys.push(reference.clone());
        if let Some(dbt) = dbt {
            if let Some(upstream) = dbt.model_by_name(reference) {
                keys.extend(lookup_keys(upstream));
            }
        }
    }
    if let Some(dbt) = dbt {
        let identity = model.identity().to_string();
        if let Some(deps) = dbt.graph.depends_on.get(&identity) {
            for dep in deps {
                keys.push(dep.clone());
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

pub fn model_for_path<'a>(dbt: &'a DbtProject, path: &std::path::Path) -> Option<&'a DbtModel> {
    dbt.models
        .values()
        .find(|model| model.path.as_deref() == Some(path))
}

pub fn model_identity(model: &DbtModel) -> String {
    model
        .unique_id
        .clone()
        .or_else(|| model.node_id.clone())
        .unwrap_or_else(|| model.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_dbt::DbtModel;

    #[test]
    fn warehouse_priors_differ_for_bigquery_vs_snowflake() {
        let model = DbtModel {
            name: "fct".into(),
            partition_by: Some("dt".into()),
            ..DbtModel::default()
        };
        let base = Estimate::from_point(100.0, Some(0.5));
        let bq = apply_dbt_scan_priors(&model, base, Warehouse::BigQuery);
        let sf = apply_dbt_scan_priors(&model, base, Warehouse::Snowflake);
        assert!((bq.median() - 50.0).abs() < 0.01);
        assert!((sf.median() - 75.0).abs() < 0.01);
        assert!(bq.median() < sf.median());
    }

    #[test]
    fn size_prior_when_no_inputs() {
        let config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        let ctx = VolumeContext {
            config: &config,
            observations: None,
            catalog: None,
            query_history: None,
            dbt: None,
        };
        let model = DbtModel {
            name: "fct_orders".into(),
            ..DbtModel::default()
        };
        let vol = ctx.resolve_for_model(&model);
        assert_eq!(vol.grade, CostGrade::C);
        assert!(vol.bytes.median() > 0.0);
    }

    #[test]
    fn incremental_discount_preserves_sigma() {
        let config = CostConfig {
            enabled: true,
            incremental_fraction: 0.05,
            ..CostConfig::default()
        };
        let ctx = VolumeContext {
            config: &config,
            observations: None,
            catalog: None,
            query_history: None,
            dbt: None,
        };
        let model = DbtModel {
            name: "fct".into(),
            materialized: Some("incremental".into()),
            unique_key: Some("id".into()),
            ..DbtModel::default()
        };
        let base = Estimate::from_point(100.0, Some(0.5));
        let sigma_before = base.sigma;
        let discounted = ctx.apply_incremental(&model, base);
        assert!((discounted.sigma - sigma_before).abs() < 1e-9);
        assert!((discounted.median() - 5.0).abs() < 0.01);
    }

    #[test]
    fn unbounded_incremental_rule_skips_discount() {
        let config = CostConfig::default();
        let ctx = VolumeContext {
            config: &config,
            observations: None,
            catalog: None,
            query_history: None,
            dbt: None,
        };
        let model = DbtModel {
            name: "fct".into(),
            materialized: Some("incremental".into()),
            unique_key: Some("id".into()),
            ..DbtModel::default()
        };
        let with_rule = ctx.resolve_for_model_with_rule(&model, Some("SQLCOST019"));
        let without_rule = ctx.resolve_for_model(&model);
        assert!(with_rule.bytes.median() > without_rule.bytes.median());
    }

    #[test]
    fn catalog_rows_fallback() {
        use crate::catalog::CatalogStats;
        let mut catalog = CatalogStats::default();
        catalog
            .rows_by_key
            .insert("model.proj.fct".into(), 1_000_000);
        let config = CostConfig::default();
        let ctx = VolumeContext {
            config: &config,
            observations: None,
            catalog: Some(&catalog),
            query_history: None,
            dbt: None,
        };
        let model = DbtModel {
            unique_id: Some("model.proj.fct".into()),
            name: "fct".into(),
            ..DbtModel::default()
        };
        let vol = ctx.resolve_for_model(&model);
        assert_eq!(vol.grade, CostGrade::B);
        assert!((vol.bytes.median() - 200_000_000.0).abs() < 1.0);
    }

    #[test]
    fn query_history_is_measured_without_priors() {
        use crate::query_history::{QueryHistoryEntry, QueryHistoryStats};
        let mut history = QueryHistoryStats::default();
        history.by_key.insert(
            "fct".into(),
            QueryHistoryEntry {
                bytes_per_run: 1_000_000_000.0,
                runs_per_month: 30.0,
            },
        );
        let config = CostConfig::default();
        let ctx = VolumeContext {
            config: &config,
            observations: None,
            catalog: None,
            query_history: Some(&history),
            dbt: None,
        };
        let model = DbtModel {
            name: "fct".into(),
            materialized: Some("view".into()),
            partition_by: Some("dt".into()),
            ..DbtModel::default()
        };
        let vol = ctx.resolve_for_model(&model);
        assert!(vol.measured);
        assert_eq!(vol.estimation_basis, "query-history");
        assert!((vol.bytes.median() - 1_000_000_000.0).abs() < 1.0);
    }
}

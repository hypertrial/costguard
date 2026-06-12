use crate::catalog::CatalogStats;
use crate::config::{parse_bytes_spec, CostConfig, CostSourceOverride};
use crate::query_history::QueryHistoryStats;
use crate::Estimate;
use costguard_dbt::{DbtModel, DbtProject};
use costguard_diagnostics::CostGrade;

pub struct ResolvedVolume {
    pub bytes: Estimate,
    pub runs_per_month: Estimate,
    pub grade: CostGrade,
}

pub struct VolumeContext<'a> {
    pub config: &'a CostConfig,
    pub catalog: Option<&'a CatalogStats>,
    pub query_history: Option<&'a QueryHistoryStats>,
    pub dbt: Option<&'a DbtProject>,
}

impl VolumeContext<'_> {
    pub fn resolve_for_model(&self, model: &DbtModel) -> ResolvedVolume {
        let runs = self.runs_for_model(model);
        if let Some(history) = self.query_history {
            for key in lookup_keys(model) {
                if let Some(entry) = history.by_key.get(&key) {
                    return ResolvedVolume {
                        bytes: Estimate::from_point(entry.bytes_per_run, Some(0.2)),
                        runs_per_month: Estimate::from_point(entry.runs_per_month, Some(0.1)),
                        grade: CostGrade::A,
                    };
                }
            }
        }

        if let Some(catalog) = self.catalog {
            if let Some(unique_id) = model.unique_id.as_ref() {
                if let Some(bytes) = catalog.bytes_by_key.get(unique_id) {
                    let mut est = Estimate::from_point(*bytes as f64, Some(0.25));
                    est = self.apply_incremental(model, est);
                    return ResolvedVolume {
                        bytes: est,
                        runs_per_month: runs,
                        grade: CostGrade::B,
                    };
                }
            }
        }

        if let Some(source) = self.resolve_config_source(model) {
            return source;
        }

        let prior_bytes = self.config.default_table_size.default_bytes();
        let mut est = Estimate::from_point(prior_bytes, Some(0.8));
        est = self.apply_incremental(model, est);
        ResolvedVolume {
            bytes: est,
            runs_per_month: runs,
            grade: CostGrade::C,
        }
    }

    fn runs_for_model(&self, model: &DbtModel) -> Estimate {
        for key in lookup_keys(model) {
            if let Some(override_cfg) = self.config.models.get(&key) {
                if let Some(runs) = override_cfg.runs_per_month {
                    return Estimate::from_point(runs, Some(0.1));
                }
            }
        }
        Estimate::from_point(self.config.default_runs_per_month, Some(0.2))
    }

    fn apply_incremental(&self, model: &DbtModel, bytes: Estimate) -> Estimate {
        if model.materialized.as_deref() == Some("incremental") && model.unique_key.is_some() {
            let frac = self.config.incremental_fraction;
            Estimate::from_point(bytes.median() * frac, Some(0.3))
        } else {
            bytes
        }
    }

    fn resolve_config_source(&self, model: &DbtModel) -> Option<ResolvedVolume> {
        let keys = upstream_keys(model, self.dbt);
        for key in keys {
            if let Some(source) = self.config.sources.get(&key) {
                if let Some(bytes) = source_bytes_estimate(source) {
                    let runs = self.runs_for_model(model);
                    let mut est = bytes;
                    est = self.apply_incremental(model, est);
                    return Some(ResolvedVolume {
                        bytes: est,
                        runs_per_month: runs,
                        grade: CostGrade::B,
                    });
                }
            }
        }
        None
    }
}

fn source_bytes_estimate(source: &CostSourceOverride) -> Option<Estimate> {
    if let Some(bytes) = &source.bytes {
        let value = parse_bytes_spec(bytes).ok()?;
        return Some(Estimate::from_point(value, Some(0.3)));
    }
    if let Some(rows) = &source.rows {
        let row_est = super::config::range_or_point_to_estimate(rows, Some(0.3));
        let avg = source.avg_row_bytes.unwrap_or(200.0);
        return Some(Estimate::from_point(row_est.median() * avg, Some(0.4)));
    }
    None
}

fn lookup_keys(model: &DbtModel) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(id) = &model.unique_id {
        keys.push(id.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_dbt::DbtModel;

    #[test]
    fn size_prior_when_no_inputs() {
        let config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        let ctx = VolumeContext {
            config: &config,
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
}

use crate::baseline::{
    load_finding_baseline, validate_baseline_policy_digest, validate_finding_baseline,
    write_finding_baseline,
};
use crate::config::ScanConfig;
use crate::governance::{apply_managed_governance, normalized_path, ManagedPolicy};
use crate::owners::{assign_diagnostic_owners, OwnerResolver};
use crate::pipeline::{
    apply_baseline_filter, apply_enabled_and_severity, apply_inline_suppressions,
    assign_semantic_identities, effective_rule_overrides, suppression_scope,
    validate_rule_registration, IdentityResult, SuppressionScope,
};
use crate::sql_analysis::{analyze_sql_documents, dbt_models_by_path, parse_failure_diagnostics};
use crate::waivers::apply_local_waivers;
use crate::{AnalysisViolation, LoadedProject};
use anyhow::Result;
use costguard_cost::{
    is_infrastructure_rule, run_cost_analysis_for_project, summarize_features, CostAnalysisResult,
    CostInputs, ModelFeatureSummary,
};
use costguard_dbt::compiled_code_by_model_path;
use costguard_protocol::EnforcementOutcome;
use costguard_rules::{ProjectIndexes, RuleRegistry};
use costguard_scanner::ProjectFile;
use costguard_sql::{JoinKind, SqlDocument};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(crate) struct BranchDiagnostics {
    pub diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    pub sql_documents: Vec<SqlDocument>,
    pub target_documents: Vec<SqlDocument>,
    pub policy_violations: Vec<costguard_policy::PolicyViolation>,
    pub waiver_violations: Vec<AnalysisViolation>,
    pub baselined_findings: usize,
    pub cost_analysis: Option<CostAnalysisResult>,
}

pub(crate) struct BranchScanInput<'a> {
    pub config: &'a ScanConfig,
    pub root: &'a Path,
    pub owner_resolver: &'a OwnerResolver,
    pub managed_policy: Option<&'a ManagedPolicy>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub baseline: Option<&'a crate::FindingBaseline>,
    pub policy_digest: &'a str,
}

pub(crate) struct BranchScanRun<'a> {
    pub project: &'a LoadedProject,
    pub analysis_files: &'a [ProjectFile],
    pub metadata_diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    pub write_baseline_path: Option<&'a Path>,
}

impl<'a> BranchScanInput<'a> {
    pub fn with_scan(self, run: BranchScanRun<'a>) -> BranchScanRequest<'a> {
        BranchScanRequest { input: self, run }
    }
}

pub(crate) struct BranchScanRequest<'a> {
    input: BranchScanInput<'a>,
    run: BranchScanRun<'a>,
}

pub(crate) fn run_branch_diagnostics(request: BranchScanRequest<'_>) -> Result<BranchDiagnostics> {
    let BranchScanRequest { input, run } = request;
    let BranchScanInput {
        config,
        root,
        owner_resolver,
        managed_policy,
        started_at,
        baseline,
        policy_digest,
    } = input;
    let BranchScanRun {
        project,
        analysis_files,
        metadata_diagnostics,
        write_baseline_path,
    } = run;
    let compiled_by_path = project
        .dbt
        .as_ref()
        .map(compiled_code_by_model_path)
        .unwrap_or_default();
    let sql_documents = analyze_sql_documents(analysis_files, config.platform, &compiled_by_path);
    let project_indexes = ProjectIndexes::from_sql_documents(&sql_documents);
    let sql_by_path = sql_documents
        .iter()
        .map(|doc| (doc.path.clone(), doc))
        .collect::<HashMap<_, _>>();
    let dbt_by_path = dbt_models_by_path(project);
    let registry = RuleRegistry::default_rules();
    validate_rule_registration(&registry)?;

    let mut raw_diagnostics = metadata_diagnostics;
    let file_texts = file_text_map(&project.files, &project.compiled_unmapped_paths);
    let file_scopes = file_scope_map(&project.files);
    for file in &project.files {
        let sql = sql_by_path.get(&file.path).copied();
        let dbt_model = dbt_by_path.get(&file.root_relative_path).copied();
        let policy_path = normalized_path(&file.root_relative_path);
        let resolved = managed_policy
            .map(|policy| policy.resolve(Some(&policy_path)))
            .transpose()?;
        let overrides = effective_rule_overrides(config, resolved.as_ref());
        let ctx = costguard_rules::RuleContext {
            warehouse: config.platform,
            file,
            sql,
            dbt_model,
            all_sql: &sql_documents,
            project_indexes: &project_indexes,
            overrides: &overrides,
        };
        let framework = project
            .graph
            .model_for_path(&file.root_relative_path)
            .map(|model| model.framework)
            .or_else(|| {
                project
                    .rocky_owned_paths
                    .contains(&file.root_relative_path)
                    .then_some(costguard_project::Framework::Rocky)
            });
        let mut file_diagnostics = registry.run_for_framework(&ctx, framework);
        if let Some(policy) = &resolved {
            file_diagnostics.extend(registry.run_declarative(&ctx, &policy.custom_rules)?);
        }
        if project
            .compiled_unmapped_paths
            .contains(&file.root_relative_path)
        {
            for diagnostic in &mut file_diagnostics {
                diagnostic.compiled_line = Some(diagnostic.line);
                diagnostic.compiled_column = Some(diagnostic.column);
                diagnostic.line = 1;
                diagnostic.column = 1;
                diagnostic.span = None;
                diagnostic.source_provenance =
                    Some(costguard_diagnostics::SourceProvenance::CompiledUnmapped);
            }
        }
        raw_diagnostics.extend(apply_enabled_and_severity(file_diagnostics, &overrides));
    }

    let target_paths = project
        .files
        .iter()
        .map(|file| &file.path)
        .collect::<std::collections::HashSet<_>>();
    let target_documents = sql_documents
        .iter()
        .filter(|document| target_paths.contains(&document.path))
        .cloned()
        .collect::<Vec<_>>();
    raw_diagnostics.extend(parse_failure_diagnostics(&target_documents, root));

    let allow_inline =
        managed_policy.is_none_or(|policy| policy.document.permissions.allow_inline_suppressions);
    let diagnostics =
        apply_inline_suppressions(raw_diagnostics, &file_texts, &file_scopes, allow_inline);
    let IdentityResult { mut diagnostics } = assign_semantic_identities(diagnostics)?;
    let policy_violations = apply_managed_governance(&mut diagnostics, managed_policy, started_at)?;
    assign_diagnostic_owners(
        &mut diagnostics,
        owner_resolver,
        project.dbt.as_ref(),
        &project.graph,
    );
    let waiver_violations = apply_local_waivers(&mut diagnostics, &config.waivers, started_at);

    if let Some(path) = write_baseline_path {
        write_baseline_from_scan(config, root, &diagnostics, policy_digest, path)?;
    }
    let (diagnostics, baselined_findings) = apply_baseline_filter(diagnostics, baseline);
    let mut diagnostics = crate::pipeline::filter_by_min_confidence(
        diagnostics,
        config.min_confidence,
        config.min_confidence_filter,
    );
    for diagnostic in &mut diagnostics {
        if let Some(tier) = costguard_protocol::precision_tier(&diagnostic.rule_id) {
            diagnostic.rule_precision_tier = Some(tier.to_string());
        }
    }
    let features_by_path = feature_summaries_by_path(&sql_documents);
    let cost_analysis =
        run_optional_cost(config, root, project, &mut diagnostics, &features_by_path)?;
    Ok(BranchDiagnostics {
        diagnostics,
        sql_documents,
        target_documents,
        policy_violations,
        waiver_violations,
        baselined_findings,
        cost_analysis,
    })
}

pub(crate) fn finding_delta_diagnostics(
    diagnostics: &[costguard_diagnostics::Diagnostic],
) -> Vec<costguard_diagnostics::Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.governance.enforcement != EnforcementOutcome::Excepted
                && !is_infrastructure_rule(&diagnostic.rule_id)
        })
        .cloned()
        .collect()
}

fn write_baseline_from_scan(
    config: &ScanConfig,
    root: &Path,
    diagnostics: &[costguard_diagnostics::Diagnostic],
    policy_digest: &str,
    path: &Path,
) -> Result<()> {
    let output = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let baseline_to_write = if let Some(existing_path) = &config.baseline_path {
        let existing = load_finding_baseline(
            if existing_path.is_absolute() {
                existing_path.clone()
            } else {
                root.join(existing_path)
            }
            .as_path(),
        )?;
        validate_finding_baseline(&existing, config.platform)?;
        validate_baseline_policy_digest(&existing, policy_digest)?;
        crate::baseline::merge_baseline_findings(
            &existing,
            diagnostics,
            config.platform,
            Some(policy_digest.into()),
        )
    } else {
        crate::FindingBaseline::from_diagnostics(
            diagnostics,
            config.platform,
            Some(policy_digest.into()),
        )
    };
    write_finding_baseline(&output, &baseline_to_write)
}

fn run_optional_cost(
    config: &ScanConfig,
    root: &Path,
    project: &LoadedProject,
    diagnostics: &mut [costguard_diagnostics::Diagnostic],
    features_by_path: &HashMap<PathBuf, ModelFeatureSummary>,
) -> Result<Option<CostAnalysisResult>> {
    if let Some(cost_config) = &config.cost {
        if cost_config.enabled {
            let inputs = CostInputs::load(root, cost_config)?;
            return Ok(Some(run_cost_analysis_for_project(
                cost_config,
                project.dbt.as_ref(),
                Some(&project.graph),
                &inputs,
                diagnostics,
                features_by_path,
            )));
        }
    }
    Ok(None)
}

fn feature_summaries_by_path(documents: &[SqlDocument]) -> HashMap<PathBuf, ModelFeatureSummary> {
    documents
        .iter()
        .map(|doc| {
            let cte_reuse_max = doc
                .features
                .cte_references
                .iter()
                .map(|cte| cte.key.parse::<usize>().unwrap_or(1))
                .max()
                .unwrap_or(0);
            (
                doc.path.clone(),
                summarize_features(
                    doc.features.joins.len(),
                    doc.features
                        .joins
                        .iter()
                        .filter(|join| matches!(join.kind, JoinKind::Cross | JoinKind::Comma))
                        .count(),
                    doc.features.select_stars.len(),
                    doc.features.window_functions.len(),
                    cte_reuse_max,
                ),
            )
        })
        .collect()
}

fn file_text_map(
    files: &[ProjectFile],
    compiled_unmapped_paths: &std::collections::BTreeSet<PathBuf>,
) -> HashMap<PathBuf, String> {
    files
        .iter()
        .filter(|file| !compiled_unmapped_paths.contains(&file.root_relative_path))
        .map(|file| (file.root_relative_path.clone(), file.text.clone()))
        .collect()
}

fn file_scope_map(files: &[ProjectFile]) -> HashMap<PathBuf, SuppressionScope> {
    files
        .iter()
        .map(|file| (file.root_relative_path.clone(), suppression_scope(file)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::Severity;

    #[test]
    fn infrastructure_findings_do_not_enter_finding_deltas() {
        let metadata = costguard_diagnostics::Diagnostic::new(
            "SQLCOST046",
            Severity::Low,
            PathBuf::from("target/manifest.json"),
            None,
            "stale manifest",
        );
        let behavioral = costguard_diagnostics::Diagnostic::new(
            "SQLCOST001",
            Severity::High,
            PathBuf::from("models/a.sql"),
            None,
            "select star",
        );
        let filtered = finding_delta_diagnostics(&[metadata, behavioral]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule_id, "SQLCOST001");
    }
}

use crate::context::{ContextIssue, ContextReport};
use crate::scan_plan::ScanPlan;
use crate::sql_analysis::build_scan_metrics;
use costguard_dbt::{MetadataWarning, MetadataWarningKind};
use costguard_diagnostics::{EvidenceBuilder, Severity};
use costguard_scanner::{ScanCounts, SkippedFile};
use costguard_sql::SqlDocument;
use std::path::{Path, PathBuf};

pub(crate) fn skipped_files_all(plan: &ScanPlan) -> Vec<SkippedFile> {
    let mut all = plan.target_skips.clone();
    all.extend(plan.context_skips.clone());
    all
}

pub(crate) fn partition_metadata_warnings(
    warnings: &[MetadataWarning],
    plan: &ScanPlan,
    skipped_files: &[SkippedFile],
) -> (Vec<MetadataWarning>, Vec<MetadataWarning>) {
    let mut target = Vec::new();
    let mut context = Vec::new();
    for warning in warnings {
        if warning.kind == MetadataWarningKind::FileSkipped {
            if let Some(path) = &warning.path {
                if plan.is_target(path) {
                    target.push(warning.clone());
                } else if plan.pr_mode {
                    context.push(warning.clone());
                }
            } else {
                target.push(warning.clone());
            }
        } else {
            target.push(warning.clone());
        }
    }
    for file in skipped_files {
        let warning = MetadataWarning {
            kind: MetadataWarningKind::FileSkipped,
            path: Some(file.path.clone()),
            message: format!(
                "skipped {}: {}",
                file.root_relative_path.display(),
                file.reason
            ),
        };
        if plan.is_target(&file.path) {
            target.push(warning);
        } else if plan.pr_mode {
            context.push(warning);
        }
    }
    (target, context)
}

pub(crate) fn metadata_diagnostics(
    root: &Path,
    warnings: &[MetadataWarning],
) -> Vec<costguard_diagnostics::Diagnostic> {
    warnings
        .iter()
        .map(|warning| metadata_warning_to_diagnostic(root, warning))
        .collect()
}

fn metadata_warning_to_diagnostic(
    root: &Path,
    warning: &MetadataWarning,
) -> costguard_diagnostics::Diagnostic {
    let (rule_id, severity, suggestion, kind) = match warning.kind {
        MetadataWarningKind::NoManifest => (
            "SQLCOST023",
            Severity::Info,
            "run 'dbt compile' to produce target/manifest.json, then scan again (or pass --manifest <path>)",
            "no_manifest",
        ),
        MetadataWarningKind::StaleManifest => (
            "SQLCOST045",
            Severity::Info,
            "re-run 'dbt compile' so target/manifest.json reflects current models, then scan again",
            "stale_manifest",
        ),
        MetadataWarningKind::ManifestChecksumMismatch => (
            "SQLCOST046",
            Severity::Low,
            "re-run 'dbt compile' so the manifest checksum matches current model SQL",
            "manifest_checksum_mismatch",
        ),
        MetadataWarningKind::YamlParseFailed => (
            "SQLCOST024",
            Severity::Low,
            "fix the schema YAML syntax so Costguard can read model config",
            "yaml_parse_failed",
        ),
        MetadataWarningKind::DbtProjectParseFailed
        | MetadataWarningKind::DbtProjectAmbiguousModels => (
            "SQLCOST025",
            Severity::Low,
            "fix dbt_project.yml syntax or project name alignment for folder config",
            "dbt_project_metadata",
        ),
        MetadataWarningKind::FileSkipped => (
            "SQLCOST026",
            Severity::Low,
            "narrow scan paths, ignore generated files, or raise [scan].max_file_bytes in costguard.toml",
            "file_skipped",
        ),
    };
    let path = warning
        .path
        .as_ref()
        .map(|path| {
            path.strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone())
        })
        .unwrap_or_else(|| PathBuf::from("."));
    let subject = costguard_diagnostics::posix_path(&path);
    let evidence_key = EvidenceBuilder::new("metadata")
        .field("kind", kind)
        .field("subject", subject)
        .build();
    let mut diagnostic = costguard_diagnostics::Diagnostic::new(
        rule_id,
        severity,
        path,
        None,
        warning.message.clone(),
    )
    .with_suggestion(suggestion);
    diagnostic.governance.evidence_key = evidence_key;
    diagnostic
}

pub(crate) fn build_context_report(
    plan: &ScanPlan,
    union_sql_documents: &[SqlDocument],
    context_skip_warnings: &[MetadataWarning],
    root: &Path,
) -> ContextReport {
    let context_docs: Vec<SqlDocument> = union_sql_documents
        .iter()
        .filter(|doc| !plan.is_target(&doc.path))
        .cloned()
        .collect();
    let mut issues = Vec::new();
    for doc in &context_docs {
        if doc.is_dbt_sql_model && !doc.parsed {
            let path = doc
                .path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| doc.path.clone());
            issues.push(ContextIssue {
                rule_id: "SQLCOST027".into(),
                path: costguard_diagnostics::posix_path(&path),
                message: format!(
                    "SQL parse failed for {}",
                    doc.path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("model")
                ),
            });
        }
    }
    for warning in context_skip_warnings {
        let path = warning
            .path
            .as_ref()
            .map(|p| costguard_diagnostics::posix_path(p.strip_prefix(root).unwrap_or(p)))
            .unwrap_or_else(|| ".".into());
        issues.push(ContextIssue {
            rule_id: "SQLCOST026".into(),
            path,
            message: warning.message.clone(),
        });
    }
    let metrics = build_scan_metrics(
        &context_docs,
        &[],
        ScanCounts::from_files(&plan.context),
        false,
        0,
        0,
        0,
    );
    ContextReport {
        counts: ScanCounts::from_files(&plan.context),
        sql_parse_total: metrics.sql_parse_total,
        sql_parse_failures: metrics.sql_parse_failures,
        sql_parse_other_total: metrics.sql_parse_other_total,
        sql_parse_other_failures: metrics.sql_parse_other_failures,
        skipped_count: plan.context_skips.len(),
        issues,
    }
}

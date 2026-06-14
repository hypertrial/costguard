use crate::{FileParseStatus, Project, ScanMetrics};
use costguard_dbt::{extract_sql_features, DbtModel};
use costguard_diagnostics::{Confidence, Diagnostic, EvidenceBuilder, Severity};
use costguard_platform::Platform;
use costguard_scanner::{FileKind, ProjectFile, ScanCounts};
use costguard_sql::{analyze_sql, SqlDocument};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub(crate) fn analyze_sql_documents(
    files: &[ProjectFile],
    platform: Platform,
    compiled_by_path: &HashMap<PathBuf, String>,
) -> Vec<SqlDocument> {
    let mut documents = files
        .par_iter()
        .filter(|file| matches!(file.kind, FileKind::Sql | FileKind::DbtSqlModel))
        .map(|file| {
            let compiled_code = compiled_by_path
                .get(&file.root_relative_path)
                .map(String::as_str);
            let dbt = extract_sql_features(&file.text);
            analyze_sql(
                file.path.clone(),
                &file.text,
                platform,
                &file.line_index,
                compiled_code,
                file.kind == FileKind::DbtSqlModel,
                dbt,
            )
        })
        .collect::<Vec<_>>();
    documents.sort_by(|left, right| left.path.cmp(&right.path));
    documents
}

pub(crate) fn build_file_parse_status(sql_documents: &[SqlDocument]) -> Vec<FileParseStatus> {
    sql_documents
        .iter()
        .filter(|doc| doc.is_dbt_sql_model)
        .map(|doc| FileParseStatus {
            path: doc.path.clone(),
            parse_input: doc.parse_input,
            parsed: doc.parsed,
            parsed_raw: doc.parsed_raw,
            parsed_compiled: doc.parsed_compiled,
            feature_extraction_used_ast: doc.feature_extraction_used_ast,
        })
        .collect()
}

pub(crate) fn parse_failure_diagnostics(
    sql_documents: &[SqlDocument],
    root: &Path,
) -> Vec<Diagnostic> {
    sql_documents
        .iter()
        .filter(|doc| doc.is_dbt_sql_model && !doc.parsed)
        .map(|doc| {
            let path = doc
                .path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| doc.path.clone());
            let filename = doc
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("model");
            let evidence_key = EvidenceBuilder::new("parse_failure")
                .field("file", filename)
                .build();
            let mut diagnostic = Diagnostic::new(
                "SQLCOST027",
                Severity::Info,
                path,
                None,
                format!(
                    "SQL parse failed for {}; rules may use regex fallback with lower confidence",
                    filename
                ),
            )
            .with_suggestion(
                "run dbt compile and pass --manifest target/manifest.json for compiled SQL fallback",
            )
            .with_confidence(Confidence::High);
            diagnostic.governance.evidence_key = evidence_key;
            diagnostic
        })
        .collect()
}

pub(crate) fn build_scan_metrics(
    sql_documents: &[SqlDocument],
    diagnostics: &[Diagnostic],
    counts: ScanCounts,
    metadata_only: bool,
    metadata_warnings: usize,
    yaml_parse_failures: usize,
    dbt_project_parse_failures: usize,
) -> ScanMetrics {
    let model_docs = sql_documents
        .iter()
        .filter(|doc| doc.is_dbt_sql_model)
        .collect::<Vec<_>>();
    let other_docs = sql_documents
        .iter()
        .filter(|doc| !doc.is_dbt_sql_model)
        .collect::<Vec<_>>();
    let compiled_docs = model_docs
        .iter()
        .filter(|doc| doc.used_compiled_for_parse)
        .copied()
        .collect::<Vec<_>>();

    let sql_parse_total = model_docs.len();
    let sql_parse_failures = model_docs.iter().filter(|doc| !doc.parsed).count();
    let sql_parse_other_total = other_docs.len();
    let sql_parse_other_failures = other_docs.iter().filter(|doc| !doc.parsed).count();
    let sql_parse_compiled_total = compiled_docs.len();
    let sql_parse_compiled_failures = compiled_docs
        .iter()
        .filter(|doc| !doc.parsed_compiled)
        .count();
    let mut diagnostics_by_rule = BTreeMap::new();
    let mut diagnostics_by_severity = BTreeMap::new();
    for diagnostic in diagnostics {
        *diagnostics_by_rule
            .entry(diagnostic.rule_id.clone())
            .or_default() += 1;
        *diagnostics_by_severity
            .entry(diagnostic.severity.label().to_ascii_lowercase())
            .or_default() += 1;
    }
    ScanMetrics {
        counts,
        sql_parse_total,
        sql_parse_failures,
        sql_parse_other_total,
        sql_parse_other_failures,
        sql_parse_compiled_total,
        sql_parse_compiled_failures,
        metadata_warnings,
        yaml_parse_failures,
        dbt_project_parse_failures,
        metadata_only_scan: metadata_only,
        diagnostics_by_rule,
        diagnostics_by_severity,
        baselined_findings: 0,
        new_findings: diagnostics.len(),
    }
}

pub(crate) fn dbt_models_by_path(project: &Project) -> HashMap<PathBuf, &DbtModel> {
    project
        .dbt
        .as_ref()
        .into_iter()
        .flat_map(|dbt| dbt.models.values())
        .filter_map(|model| model.path.as_ref().map(|path| (path.clone(), model)))
        .collect()
}

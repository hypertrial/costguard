use crate::Platform;
use costguard_diagnostics::SourceProvenance;
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use crate::features;
use crate::strip;
use crate::SqlFeatures;

use super::normalize::{
    apply_trino_relaxed_rewrites, largest_query_statement, normalize_for_parse,
};

pub(crate) fn compiled_ast_features(
    compiled: &str,
    platform: Platform,
    regex_features: SqlFeatures,
) -> (SqlFeatures, bool) {
    let dialect = platform.sqlparser_dialect();
    let normalized = normalize_for_parse(compiled, platform);
    let normalized_index = costguard_diagnostics::LineIndex::new(&normalized);
    let strip_map = strip::JinjaStripMap::identity(normalized.len());
    let Ok(stmts) = Parser::parse_sql(dialect.as_ref(), &normalized) else {
        return (regex_features, false);
    };
    let ast_features = features::extract_shape_features_ast(
        &stmts,
        &normalized,
        &normalized,
        &strip_map,
        &normalized_index,
    );
    let mut regex_features = regex_features;
    regex_features.cte_references.clear();
    (
        features::merge_shape_features(
            regex_features,
            mark_compiled_unmapped(ast_features),
            true,
            false,
        ),
        true,
    )
}

fn mark_compiled_unmapped(mut features: SqlFeatures) -> SqlFeatures {
    for feature in features
        .select_stars
        .iter_mut()
        .chain(features.order_by_clauses.iter_mut())
        .chain(features.group_by_clauses.iter_mut())
        .chain(features.distincts.iter_mut())
        .chain(features.json_extractions.iter_mut())
        .chain(features.regex_calls.iter_mut())
        .chain(features.normalization_calls.iter_mut())
        .chain(features.cte_references.iter_mut())
        .chain(features.non_sargable_predicates.iter_mut())
        .chain(features.unions_without_all.iter_mut())
        .chain(features.count_distincts.iter_mut())
        .chain(features.wildcard_table_scans.iter_mut())
        .chain(features.correlated_subqueries.iter_mut())
        .chain(features.leading_wildcard_likes.iter_mut())
        .chain(features.or_partition_predicates.iter_mut())
        .chain(features.scalar_subqueries_in_select.iter_mut())
    {
        feature.span = feature
            .span
            .with_source_provenance(SourceProvenance::CompiledUnmapped);
    }
    for join in &mut features.joins {
        join.span = join
            .span
            .with_source_provenance(SourceProvenance::CompiledUnmapped);
    }
    for window in &mut features.window_functions {
        window.span = window
            .span
            .with_source_provenance(SourceProvenance::CompiledUnmapped);
    }
    for cte in &mut features.ctes {
        cte.span = cte
            .span
            .with_source_provenance(SourceProvenance::CompiledUnmapped);
    }
    features
}

fn try_parse_sql_error(
    dialect: &dyn sqlparser::dialect::Dialect,
    text: &str,
) -> Result<(), String> {
    Parser::parse_sql(dialect, text)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// Returns whether compiled SQL parses after Trino normalization and fallbacks.
pub fn try_parse_compiled_sql(code: &str, platform: Platform) -> bool {
    try_parse_compiled_sql_error(code, platform).is_ok()
}

/// Returns the first parse error for compiled SQL, if any.
pub fn try_parse_compiled_sql_error(code: &str, platform: Platform) -> Result<(), String> {
    let primary = platform.sqlparser_dialect();
    let normalized = normalize_for_parse(code, platform);
    if let Ok(()) = try_parse_with_dialects(primary.as_ref(), &normalized, platform) {
        return Ok(());
    }

    for relaxed in trino_relaxed_candidates(code, platform) {
        if let Ok(()) = try_parse_with_dialects(primary.as_ref(), &relaxed, platform) {
            return Ok(());
        }
    }

    if let Some(statement) = largest_query_statement(code) {
        let normalized_statement = normalize_for_parse(&statement, platform);
        if let Ok(()) = try_parse_with_dialects(primary.as_ref(), &normalized_statement, platform) {
            return Ok(());
        }
        for relaxed in trino_relaxed_candidates(&statement, platform) {
            if let Ok(()) = try_parse_with_dialects(primary.as_ref(), &relaxed, platform) {
                return Ok(());
            }
        }
    }

    let generic = GenericDialect {};
    try_parse_sql_error(&generic, &normalized)
}

fn try_parse_with_dialects(
    primary: &dyn sqlparser::dialect::Dialect,
    text: &str,
    platform: Platform,
) -> Result<(), String> {
    if let Ok(()) = try_parse_sql_error(primary, text) {
        return Ok(());
    }

    if platform == Platform::Trino {
        let generic = GenericDialect {};
        try_parse_sql_error(&generic, text)?;
    }

    Err("parse failed".to_string())
}

fn trino_relaxed_candidates(code: &str, platform: Platform) -> Vec<String> {
    if platform != Platform::Trino {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let normalized = normalize_for_parse(code, platform);
    let relaxed = apply_trino_relaxed_rewrites(&normalized);
    if relaxed != normalized {
        candidates.push(relaxed);
    }
    candidates
}

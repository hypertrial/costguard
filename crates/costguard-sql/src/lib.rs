//! SQL parsing and feature extraction.
//!
//! Strips Jinja, parses SQL with sqlparser using the warehouse
//! [`Platform`] dialect, and extracts shape
//! features (joins, CTEs, windows, etc.) used by the rule engine.

use costguard_dbt::DbtSqlFeatures;
use costguard_diagnostics::Span;
use serde::{Deserialize, Serialize};
use sqlparser::parser::Parser;
use std::path::PathBuf;

mod features;
mod parse;
mod platform;
mod strip;
mod trino;

pub use platform::Platform;

pub use parse::{normalize_for_parse, try_parse_compiled_sql, try_parse_compiled_sql_error};

/// Which SQL representation was used for parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseInput {
    Raw,
    Compiled,
    CompiledWithRawFallback,
}

#[derive(Debug, Clone)]
pub struct SqlDocument {
    pub path: PathBuf,
    pub raw_sql: String,
    pub platform: Platform,
    pub is_dbt_sql_model: bool,
    pub parsed: bool,
    pub parsed_raw: bool,
    pub parsed_compiled: bool,
    pub used_compiled_for_parse: bool,
    pub parse_input: ParseInput,
    pub features: SqlFeatures,
    pub dbt: DbtSqlFeatures,
    pub feature_extraction_used_ast: bool,
}

/// Extracted SQL shape features used by the rule engine.
#[derive(Debug, Clone, Default)]
pub struct SqlFeatures {
    pub select_stars: Vec<ExpressionFeature>,
    pub order_by_clauses: Vec<ExpressionFeature>,
    pub distincts: Vec<ExpressionFeature>,
    pub joins: Vec<JoinFeature>,
    pub window_functions: Vec<WindowFeature>,
    pub json_extractions: Vec<ExpressionFeature>,
    pub regex_calls: Vec<ExpressionFeature>,
    pub normalization_calls: Vec<ExpressionFeature>,
    pub ctes: Vec<CteFeature>,
    pub cte_references: Vec<ExpressionFeature>,
    pub non_sargable_predicates: Vec<ExpressionFeature>,
    pub unions_without_all: Vec<ExpressionFeature>,
    pub count_distincts: Vec<ExpressionFeature>,
    pub wildcard_table_scans: Vec<ExpressionFeature>,
    pub correlated_subqueries: Vec<ExpressionFeature>,
    pub leading_wildcard_likes: Vec<ExpressionFeature>,
    pub or_partition_predicates: Vec<ExpressionFeature>,
    pub scalar_subqueries_in_select: Vec<ExpressionFeature>,
    pub row_explosions: Vec<ExpressionFeature>,
    pub not_in_subqueries: Vec<ExpressionFeature>,
    pub recursive_ctes: Vec<ExpressionFeature>,
}

#[derive(Debug, Clone)]
pub struct ExpressionFeature {
    pub span: Span,
    pub text: String,
    pub key: String,
}

/// A detected join with kind, predicate, and risk flags.
#[derive(Debug, Clone)]
pub struct JoinFeature {
    pub span: Span,
    pub kind: JoinKind,
    pub predicate: Option<String>,
    pub has_equality: bool,
    pub function_on_join_key: bool,
    pub pattern_matching: bool,
    pub cross_catalog: bool,
    pub right_relation: Option<String>,
    pub equality_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
    Comma,
}

#[derive(Debug, Clone)]
pub struct WindowFeature {
    pub span: Span,
    pub text: String,
    pub has_partition_by: bool,
    pub unbounded_frame: bool,
}

/// A common table expression (CTE) definition.
#[derive(Debug, Clone)]
pub struct CteFeature {
    pub name: String,
    pub span: Span,
}

pub fn analyze_sql(
    path: PathBuf,
    text: &str,
    platform: Platform,
    line_index: &costguard_diagnostics::LineIndex,
    compiled_code: Option<&str>,
    is_dbt_sql_model: bool,
    dbt: DbtSqlFeatures,
) -> SqlDocument {
    let dialect = platform.sqlparser_dialect();
    let (sanitized, strip_map) = strip::strip_jinja_with_map(text);
    let statements = Parser::parse_sql(dialect.as_ref(), &sanitized).ok();
    let parsed_raw = statements.is_some();
    let parsed_compiled = compiled_code
        .map(|code| try_parse_compiled_sql(code, platform))
        .unwrap_or(false);
    let used_compiled_for_parse = compiled_code.is_some();
    let (parsed, parse_input) = if used_compiled_for_parse {
        if parsed_compiled {
            (true, ParseInput::Compiled)
        } else if parsed_raw {
            (true, ParseInput::CompiledWithRawFallback)
        } else {
            (false, ParseInput::Compiled)
        }
    } else {
        (parsed_raw, ParseInput::Raw)
    };
    let regex_features = features::extract_features(text, &sanitized, line_index, parsed_raw);
    let (features, feature_extraction_used_ast) = if let Some(stmts) = &statements {
        let ast_features =
            features::extract_shape_features_ast(stmts, &sanitized, text, &strip_map, line_index);
        (
            features::merge_shape_features(regex_features, ast_features, parsed_raw, true),
            parsed_raw,
        )
    } else if parsed_compiled {
        parse::compiled_ast_features(compiled_code.unwrap_or_default(), platform, regex_features)
    } else {
        (regex_features, false)
    };
    SqlDocument {
        path,
        raw_sql: text.to_string(),
        platform,
        is_dbt_sql_model,
        parsed,
        parsed_raw,
        parsed_compiled,
        used_compiled_for_parse,
        parse_input,
        features,
        dbt,
        feature_extraction_used_ast,
    }
}

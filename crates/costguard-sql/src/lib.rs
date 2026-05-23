mod features;
mod strip;

use costguard_dbt::{extract_sql_features, DbtSqlFeatures};
use costguard_diagnostics::Span;
use costguard_platform::Platform;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub use costguard_platform::Platform as SqlDialect;

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
}

#[derive(Debug, Clone)]
pub struct ExpressionFeature {
    pub span: Span,
    pub text: String,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct JoinFeature {
    pub span: Span,
    pub kind: JoinKind,
    pub predicate: Option<String>,
    pub has_equality: bool,
    pub function_on_join_key: bool,
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
}

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
) -> SqlDocument {
    let dialect = platform.sqlparser_dialect();
    let (sanitized, strip_map) = strip::strip_jinja_with_map(text);
    let parsed_raw = try_parse_sql(dialect.as_ref(), &sanitized);
    let statements = if parsed_raw {
        Parser::parse_sql(dialect.as_ref(), &sanitized).ok()
    } else {
        None
    };
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
    let dbt = extract_sql_features(text);
    let regex_features = extract_features(text, &sanitized, line_index, parsed_raw);
    let (features, feature_extraction_used_ast) = if let Some(stmts) = &statements {
        let ast_features =
            features::extract_shape_features_ast(stmts, &sanitized, text, &strip_map, line_index);
        (
            features::merge_shape_features(regex_features, ast_features, parsed_raw),
            parsed_raw,
        )
    } else if parsed_compiled {
        compiled_ast_features(compiled_code.unwrap_or_default(), platform, regex_features)
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

fn compiled_ast_features(
    compiled: &str,
    platform: Platform,
    regex_features: SqlFeatures,
) -> (SqlFeatures, bool) {
    let dialect = platform.sqlparser_dialect();
    let normalized = normalize_for_parse(compiled, platform);
    let compiled_index = costguard_diagnostics::LineIndex::new(compiled);
    let strip_map = strip::JinjaStripMap::identity(normalized.len());
    let Ok(stmts) = Parser::parse_sql(dialect.as_ref(), &normalized) else {
        return (regex_features, false);
    };
    let ast_features = features::extract_shape_features_ast(
        &stmts,
        &normalized,
        compiled,
        &strip_map,
        &compiled_index,
    );
    (
        features::merge_shape_features(regex_features, ast_features, true),
        true,
    )
}

fn try_parse_sql(dialect: &dyn sqlparser::dialect::Dialect, text: &str) -> bool {
    Parser::parse_sql(dialect, text).is_ok()
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

pub fn normalize_for_parse(text: &str, platform: Platform) -> String {
    let trimmed = text.trim();
    let without_comments = strip_sql_comments(trimmed);
    if platform == Platform::Trino {
        apply_trino_parse_rewrites(&without_comments)
    } else {
        without_comments
    }
}

fn strip_sql_comments(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut block_depth = 0usize;

    while let Some(ch) = chars.next() {
        if block_depth > 0 {
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_depth += 1;
                continue;
            }
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_depth -= 1;
            }
            continue;
        }

        if !in_single && !in_double && ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block_depth = 1;
            output.push(' ');
            continue;
        }

        if !in_single && !in_double && ch == '-' && chars.peek() == Some(&'-') {
            while chars.next().is_some_and(|next| next != '\n') {}
            output.push('\n');
            continue;
        }

        if !in_double && ch == '\'' {
            in_single = !in_single;
            output.push(ch);
            continue;
        }

        if !in_single && ch == '"' {
            in_double = !in_double;
            output.push(ch);
            continue;
        }

        output.push(ch);
    }

    output.trim().to_string()
}

fn apply_trino_parse_rewrites(text: &str) -> String {
    let without_try_cast = try_cast_regex().replace_all(text, "CAST(");
    let without_try_fn = try_fn_regex()
        .replace_all(&without_try_cast, "(")
        .to_string();
    let without_omit = omit_quotes_regex()
        .replace_all(&without_try_fn, " /* omit quotes */ ")
        .to_string();
    let without_json_null_on_error = json_null_on_error_regex()
        .replace_all(&without_omit, " /* null on error */ ")
        .to_string();
    let without_json_returning = json_value_returning_regex()
        .replace_all(&without_json_null_on_error, "")
        .to_string();
    let without_json_default = json_default_on_empty_regex()
        .replace_all(&without_json_returning, "")
        .to_string();
    let without_json_keys = json_object_key_regex()
        .replace_all(&without_json_default, "'$1',")
        .to_string();
    let without_asof = asof_left_join_regex()
        .replace_all(&without_json_keys, "LEFT JOIN")
        .to_string();
    let without_asof_join = asof_join_regex()
        .replace_all(&without_asof, "JOIN")
        .to_string();
    let without_table_fn = from_table_fn_regex()
        .replace_all(&without_asof_join, "FROM (")
        .to_string();
    let without_named_args = named_arg_regex()
        .replace_all(&without_table_fn, "")
        .to_string();
    let without_table_select = table_select_regex()
        .replace_all(&without_named_args, "(SELECT")
        .to_string();
    let without_lambda = lambda_arrow_regex()
        .replace_all(&without_table_select, " /* lambda */ (")
        .to_string();
    let without_empty_array = empty_array_regex()
        .replace_all(&without_lambda, "ARRAY[]")
        .to_string();
    let without_struct_colon = struct_field_colon_regex()
        .replace_all(&without_empty_array, "$1.$2$3")
        .to_string();
    let without_cast_dot = cast_result_dot_access_regex()
        .replace_all(&without_struct_colon, ")")
        .to_string();
    let without_map_angle = rewrite_map_angle_types(&without_cast_dot);
    let without_row_types = stub_trino_row_type_casts(&without_map_angle);
    let without_cast_row = replace_cast_row_expressions(&without_row_types);
    let with_join_alias = add_missing_join_alias_as(&without_cast_row);
    rewrite_trino_type_parens_to_angle(&with_join_alias)
}

fn apply_trino_relaxed_rewrites(text: &str) -> String {
    let with_values_json = values_json_literal_regex()
        .replace_all(text, "FROM (VALUES ('[]'))")
        .to_string();
    let with_table_alias = values_table_alias_regex()
        .replace_all(&with_values_json, ") AS $1(")
        .to_string();
    let without_wrapped_table = wrapped_quoted_table_regex()
        .replace_all(&with_table_alias, "FROM $1")
        .to_string();
    strip_returning_clause(&without_wrapped_table)
}

fn stub_trino_row_type_casts(text: &str) -> String {
    let marker = regex::Regex::new(r"(?i)\bas\s+row\s*\(").expect("valid row type marker regex");
    let mut output = String::with_capacity(text.len());
    let mut last = 0usize;
    for matched in marker.find_iter(text) {
        output.push_str(&text[last..matched.start()]);
        output.push_str("AS VARCHAR");
        last = matched.end() - 1 + skip_balanced_group(&text[matched.end() - 1..]);
    }
    output.push_str(&text[last..]);
    output
}

fn skip_balanced_group(text: &str) -> usize {
    let mut depth = 0usize;
    let mut consumed = 0usize;
    for ch in text.chars() {
        consumed += ch.len_utf8();
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    consumed
}

fn replace_cast_row_expressions(text: &str) -> String {
    let marker =
        regex::Regex::new(r"(?i)\bcast\s*\(\s*row\s*\(").expect("valid cast row marker regex");
    let mut output = String::with_capacity(text.len());
    let mut last = 0usize;
    for matched in marker.find_iter(text) {
        output.push_str(&text[last..matched.start()]);
        let mut index = matched.end() - 1 + skip_balanced_group(&text[matched.end() - 1..]);
        if let Some(rest) = text.get(index..) {
            let trimmed = rest.trim_start();
            if trimmed.len() >= 3 && trimmed[..3].eq_ignore_ascii_case("as ") {
                if let Some(close_paren) = trimmed.find(')') {
                    index += rest.len() - trimmed.len() + close_paren + 1;
                }
            }
        }
        while let Some(rest) = text.get(index..) {
            let trimmed = rest.trim_start();
            if !trimmed.starts_with('.') {
                break;
            }
            let ident_len = 1 + trimmed[1..]
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .unwrap_or(trimmed.len().saturating_sub(1));
            index += rest.len() - trimmed.len() + ident_len;
        }
        output.push_str("CAST(NULL AS VARCHAR)");
        last = index;
    }
    output.push_str(&text[last..]);
    output
}

fn add_missing_join_alias_as(text: &str) -> String {
    join_alias_missing_as_regex()
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            if prefix.trim_end().to_ascii_lowercase().ends_with(" as") {
                return caps
                    .get(0)
                    .map(|m| m.as_str())
                    .unwrap_or_default()
                    .to_string();
            }
            format!(
                "{}AS {}{}",
                prefix,
                caps.get(2).map(|m| m.as_str()).unwrap_or_default(),
                caps.get(3).map(|m| m.as_str()).unwrap_or_default()
            )
        })
        .to_string()
}

fn rewrite_map_angle_types(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let lower = text.to_ascii_lowercase();
    let mut index = 0usize;
    while index < text.len() {
        if lower[index..].starts_with("map<") {
            output.push_str("MAP(");
            index += 4;
            let (inner, consumed) = read_until_angle_close(&text[index..]);
            output.push_str(inner);
            output.push(')');
            index += consumed;
            continue;
        }
        output.push(text[index..].chars().next().expect("char"));
        index += text[index..].chars().next().expect("char").len_utf8();
    }
    output
}

fn read_until_angle_close(text: &str) -> (&str, usize) {
    let mut depth = 1usize;
    let mut consumed = 0usize;
    for ch in text.chars() {
        consumed += ch.len_utf8();
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    let inner = &text[..consumed - ch.len_utf8()];
                    return (inner, consumed);
                }
            }
            _ => {}
        }
    }
    (text, text.len())
}

fn rewrite_trino_type_parens_to_angle(text: &str) -> String {
    let marker =
        regex::Regex::new(r"(?i)\bAS\s+ARRAY\s*\(").expect("valid ARRAY type marker regex");
    let mut output = String::with_capacity(text.len());
    let mut last = 0usize;
    for matched in marker.find_iter(text) {
        output.push_str(&text[last..matched.start()]);
        output.push_str("AS ARRAY<");
        last = matched.end();
        let converted = convert_balanced_parens_to_angle(&text[last..]);
        output.push_str(&converted.converted);
        last += converted.consumed;
    }
    output.push_str(&text[last..]);
    output
}

struct BalancedConversion {
    converted: String,
    consumed: usize,
}

fn convert_balanced_parens_to_angle(text: &str) -> BalancedConversion {
    let mut converted = String::new();
    let mut depth = 1usize;
    let mut consumed = 0usize;

    for ch in text.chars() {
        consumed += ch.len_utf8();
        match ch {
            '(' => {
                depth += 1;
                converted.push('(');
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    converted.push('>');
                    break;
                }
                converted.push(')');
            }
            _ => converted.push(ch),
        }
    }

    BalancedConversion {
        converted,
        consumed,
    }
}

fn strip_returning_clause(text: &str) -> String {
    returning_clause_regex()
        .replace_all(text, "")
        .trim()
        .to_string()
}

fn split_sql_statements(text: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let chars = text.chars();
    let mut in_single = false;
    let mut in_double = false;

    for ch in chars {
        if !in_double && ch == '\'' {
            in_single = !in_single;
            current.push(ch);
            continue;
        }
        if !in_single && ch == '"' {
            in_double = !in_double;
            current.push(ch);
            continue;
        }
        if !in_single && !in_double && ch == ';' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                statements.push(trimmed.to_string());
            }
            current.clear();
            continue;
        }
        current.push(ch);
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }
    statements
}

fn largest_query_statement(text: &str) -> Option<String> {
    split_sql_statements(text)
        .into_iter()
        .filter(|statement| {
            let lower = statement.trim_start().to_ascii_lowercase();
            lower.starts_with("select") || lower.starts_with("with")
        })
        .max_by_key(|statement| statement.len())
}

fn try_cast_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\bTRY_CAST\s*\(")
}

fn try_fn_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\bTRY\s*\(")
}

fn omit_quotes_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\s+omit\s+quotes\b")
}

fn json_null_on_error_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\s+null\s+on\s+error\b")
}

fn json_value_returning_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r"(?i)\s+RETURNING\s+[A-Za-z_][\w<>]*(?:\s+DEFAULT\s+'(?:''|[^'])*'\s+ON\s+EMPTY)?",
    )
}

fn json_default_on_empty_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\s+DEFAULT\s+[\w.]+\s+ON\s+EMPTY\b")
}

fn json_object_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)'([^']+)'\s*:")
}

fn from_table_fn_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\bFROM\s+TABLE\s*\(")
}

fn named_arg_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\b[a-z_][\w]*\s*=>\s*")
}

fn table_select_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\bTABLE\s*\(\s*SELECT\b")
}

fn lambda_arrow_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\b[a-z_][\w]*\s*->\s*\(")
}

fn struct_field_colon_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\b([a-z_][\w]*):([a-z_][\w]*)(\s+as\b)")
}

fn cast_result_dot_access_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\)\s*\.\s*[a-z_][\w]*")
}

fn join_alias_missing_as_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r"(?i)\b((?:LEFT|RIGHT|FULL|INNER|CROSS)\s+JOIN\s+[a-z_][\w.]+\s+)([a-z_][\w]*)(\s+ON\b)",
    )
}

fn empty_array_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\bARRAY\s*\(\s*\)")
}

fn values_json_literal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?is)\bfrom\s*\(\s*values\s*'[\s\S]*?'\s*\)")
}

fn asof_left_join_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\basof\s+left\s+join\b")
}

fn asof_join_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\basof\s+join\b")
}

fn values_table_alias_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\)\s+([a-z_][\w]*)\s*\(")
}

fn wrapped_quoted_table_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)FROM\s+\(("(?:[^"]+)"(?:\."[^"]+")*)\)"#)
}

fn returning_clause_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r"(?i)\bRETURNING\b[\s\S]*$")
}

pub fn strip_jinja(text: &str) -> String {
    strip::strip_jinja(text)
}

pub(crate) fn from_clause_tables_start(lower: &str) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for pattern in [" from ", "\nfrom ", "\r\nfrom ", "\tfrom "] {
        if let Some(idx) = lower.find(pattern) {
            let start = idx + pattern.len();
            if best.is_none_or(|(best_idx, _)| idx < best_idx) {
                best = Some((idx, start));
            }
        }
    }
    if best.is_none() && lower.starts_with("from ") {
        return Some(5);
    }
    best.map(|(_, start)| start)
}

fn extract_features(
    text: &str,
    comma_text: &str,
    line_index: &costguard_diagnostics::LineIndex,
    parsed_raw: bool,
) -> SqlFeatures {
    let mut features = SqlFeatures::default();
    features.select_stars = matches_as_features(text, line_index, select_star_regex(), normalize);
    features.order_by_clauses =
        matches_as_features(text, line_index, order_by_regex(), |_| "order by".into());
    features.distincts = matches_as_features(text, line_index, distinct_regex(), |_| {
        "select distinct".into()
    });
    features.json_extractions =
        matches_as_features(text, line_index, json_regex(), normalize_json_key);
    features.regex_calls = matches_as_features(text, line_index, regex_call_regex(), normalize);
    features.normalization_calls =
        matches_as_features(text, line_index, normalization_regex(), normalize);
    features.window_functions = extract_windows(text, line_index);
    features.joins = extract_joins(text, comma_text, line_index, parsed_raw);
    features.ctes = extract_ctes(text, line_index);
    features.cte_references = extract_cte_references(text, line_index, &features.ctes);
    features.non_sargable_predicates =
        matches_as_features(text, line_index, non_sargable_predicate_regex(), normalize);
    features.unions_without_all = extract_unions_without_all(text, line_index);
    features.count_distincts =
        matches_as_features(text, line_index, count_distinct_regex(), |_| {
            "count(distinct".into()
        });
    features.wildcard_table_scans = extract_wildcard_table_scans(text, line_index);
    features
}

fn extract_wildcard_table_scans(
    text: &str,
    line_index: &costguard_diagnostics::LineIndex,
) -> Vec<ExpressionFeature> {
    if text.to_ascii_lowercase().contains("_table_suffix") {
        return Vec::new();
    }
    matches_as_features(text, line_index, wildcard_table_regex(), normalize)
}

fn matches_as_features<F>(
    text: &str,
    line_index: &costguard_diagnostics::LineIndex,
    regex: &Regex,
    key_fn: F,
) -> Vec<ExpressionFeature>
where
    F: Fn(&str) -> String,
{
    regex
        .find_iter(text)
        .map(|matched| {
            let raw = matched.as_str().to_string();
            ExpressionFeature {
                span: line_index.span(matched.start(), matched.end()),
                key: key_fn(&raw),
                text: raw,
            }
        })
        .collect()
}

fn extract_windows(
    text: &str,
    line_index: &costguard_diagnostics::LineIndex,
) -> Vec<WindowFeature> {
    window_regex()
        .captures_iter(text)
        .filter_map(|capture| {
            let matched = capture.get(0)?;
            let body = capture.name("body")?.as_str();
            Some(WindowFeature {
                span: line_index.span(matched.start(), matched.end()),
                text: matched.as_str().to_string(),
                has_partition_by: body.to_ascii_lowercase().contains("partition by"),
            })
        })
        .collect()
}

fn extract_joins(
    text: &str,
    comma_text: &str,
    line_index: &costguard_diagnostics::LineIndex,
    parsed_raw: bool,
) -> Vec<JoinFeature> {
    let join_text = if parsed_raw {
        text
    } else {
        &mask_string_literals(text)
    };
    let mut joins = Vec::new();
    for capture in join_regex().captures_iter(join_text) {
        let Some(matched) = capture.get(0) else {
            continue;
        };
        let kind = match capture
            .get(1)
            .map(|value| value.as_str().to_ascii_lowercase())
            .as_deref()
        {
            Some("left") => JoinKind::Left,
            Some("right") => JoinKind::Right,
            Some("full") => JoinKind::Full,
            Some("cross") => JoinKind::Cross,
            _ => JoinKind::Inner,
        };
        if parsed_raw && kind == JoinKind::Cross {
            continue;
        }
        let after_start = matched.end();
        let after_end = find_join_clause_end(&join_text[after_start..])
            .map(|offset| after_start + offset)
            .unwrap_or(join_text.len());
        let after = &join_text[after_start..after_end];
        if kind == JoinKind::Cross && is_exempt_cross_join_suffix(after) {
            continue;
        }
        let predicate = extract_on_predicate(after);
        let predicate_lower = predicate
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        joins.push(JoinFeature {
            span: line_index.span(matched.start(), after_end),
            kind,
            has_equality: has_equality_predicate(&predicate_lower),
            function_on_join_key: function_on_join_key(&predicate_lower),
            predicate,
        });
    }

    joins.extend(extract_comma_joins(comma_text, line_index));
    joins
}

fn is_exempt_cross_join_suffix(after: &str) -> bool {
    let lower = after.trim_start().to_ascii_lowercase();
    lower.starts_with("unnest(")
        || lower.starts_with("unnest (")
        || lower.starts_with("table(")
        || lower.starts_with("table (")
}

fn mask_string_literals(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_single = false;
    let mut in_double = false;

    for ch in text.chars() {
        if !in_double && ch == '\'' {
            in_single = !in_single;
            output.push(' ');
            continue;
        }
        if !in_single && ch == '"' {
            in_double = !in_double;
            output.push(' ');
            continue;
        }
        if in_single || in_double {
            output.push(' ');
        } else {
            output.push(ch);
        }
    }
    output
}

fn extract_comma_joins(
    text: &str,
    line_index: &costguard_diagnostics::LineIndex,
) -> Vec<JoinFeature> {
    let lower = text.to_ascii_lowercase();
    let Some(from_start) = from_clause_tables_start(&lower) else {
        return Vec::new();
    };
    let tail = text.get(from_start..).unwrap_or_default();
    if is_comma_join_false_positive(tail) {
        return Vec::new();
    }
    if let Some(comma_rel) = find_top_level_comma_after_from(tail) {
        let span_start = from_start;
        let span_end = from_start + comma_rel + 1;
        let span_text = text.get(span_start..span_end).unwrap_or_default();
        if span_text.to_ascii_lowercase().contains("source(") {
            return Vec::new();
        }
        return vec![JoinFeature {
            span: line_index.span(span_start, span_end),
            kind: JoinKind::Comma,
            predicate: None,
            has_equality: false,
            function_on_join_key: false,
        }];
    }
    Vec::new()
}

fn find_top_level_comma_after_from(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = 0usize;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        let ch = text[i..].chars().next()?;
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
        } else if ch == ',' && depth == 0 {
            let tail = text[i + 1..].trim_start();
            if tail
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic() || first == '(' || first == '_')
            {
                return Some(i);
            }
            return None;
        } else if depth == 0 {
            let lower_tail = text[i..].to_ascii_lowercase();
            if lower_tail.starts_with(" where ")
                || lower_tail.starts_with(" group by ")
                || lower_tail.starts_with(" order by ")
                || lower_tail.starts_with(" limit ")
                || lower_tail.starts_with("\nwhere ")
            {
                return None;
            }
        }
        i += ch.len_utf8();
    }
    None
}

fn is_comma_join_false_positive(text: &str) -> bool {
    is_leading_derived_comma_fp(&format!("from{text}")) || looks_like_subquery_comma_fp(text)
}

fn looks_like_subquery_comma_fp(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let trimmed = lower.trim_start();
    if !trimmed.starts_with('(') {
        return false;
    }
    let select_idx = trimmed.find("select").unwrap_or(usize::MAX);
    let comma_idx = trimmed.find(',').unwrap_or(usize::MAX);
    select_idx < comma_idx
}

fn find_join_clause_end(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    [
        " left join",
        " right join",
        " full join",
        " inner join",
        " cross join",
        " join",
        " where ",
        " group by ",
        " order by ",
        " limit ",
        "\nwhere ",
        "\ngroup by ",
        "\norder by ",
        "\nlimit ",
    ]
    .iter()
    .filter_map(|needle| lower.find(needle))
    .min()
}

fn extract_on_predicate(after: &str) -> Option<String> {
    let predicate = on_regex().name(after, "predicate")?;
    Some(predicate.as_str().trim().to_string())
}

trait RegexName {
    fn name<'a>(&self, text: &'a str, name: &str) -> Option<regex::Match<'a>>;
}

impl RegexName for Regex {
    fn name<'a>(&self, text: &'a str, name: &str) -> Option<regex::Match<'a>> {
        self.captures(text)?.name(name)
    }
}

fn has_equality_predicate(predicate: &str) -> bool {
    equality_regex().is_match(predicate) && !predicate.contains("!=") && !predicate.contains("<>")
}

fn function_on_join_key(predicate: &str) -> bool {
    let lower = predicate.to_ascii_lowercase();
    if is_symmetric_normalization_predicate(&lower) {
        return false;
    }
    function_on_join_key_regex().is_match(predicate)
}

fn is_symmetric_normalization_predicate(predicate: &str) -> bool {
    for func in ["lower", "upper", "trim", "ltrim", "rtrim"] {
        let pattern = format!(r"{func}\s*\([^)]+\)\s*=\s*{func}\s*\(");
        if Regex::new(&pattern)
            .map(|re| re.is_match(predicate))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn extract_ctes(text: &str, line_index: &costguard_diagnostics::LineIndex) -> Vec<CteFeature> {
    cte_regex()
        .captures_iter(text)
        .filter_map(|capture| {
            let matched = capture.get(1)?;
            Some(CteFeature {
                name: matched.as_str().to_ascii_lowercase(),
                span: line_index.span(matched.start(), matched.end()),
            })
        })
        .collect()
}

fn extract_cte_references(
    text: &str,
    line_index: &costguard_diagnostics::LineIndex,
    ctes: &[CteFeature],
) -> Vec<ExpressionFeature> {
    ctes.iter()
        .flat_map(|cte| {
            cte_name_regex(&cte.name)
                .find_iter(text)
                .filter(move |matched| matched.start() != cte.span.byte_start)
                .map(move |matched| ExpressionFeature {
                    span: line_index.span(matched.start(), matched.end()),
                    text: matched.as_str().to_string(),
                    key: cte.name.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn cte_name_regex(name: &str) -> &'static Regex {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().expect("cte regex cache");
    let key = name.to_ascii_lowercase();
    if let Some(regex) = cache.get(&key) {
        return regex;
    }
    let regex = Box::leak(Box::new(
        Regex::new(&format!(r#"(?i)\b{}\b"#, regex::escape(name))).expect("valid cte regex"),
    ));
    cache.insert(key, regex);
    regex
}

fn normalize(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn normalize_json_key(raw: &str) -> String {
    let lowered = normalize(raw);
    if let Some(before_paren) = lowered.split('(').nth(1) {
        let first_arg = before_paren.split(',').next().unwrap_or(before_paren);
        return first_arg.trim().to_string();
    }
    lowered
}

fn cached_regex(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

fn select_star_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)\bselect\s+(?:distinct\s+)?(?:[\w"]+\.)?\*"#)
}

fn order_by_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)\border\s+by\b"#)
}

fn distinct_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)\bselect\s+distinct\b"#)
}

fn json_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r#"(?i)\b(json_extract(?:_scalar)?|json_value|json_query|parse_json|get_path)\s*\([^)]*\)|\b\w+\s*:\s*["']?[A-Za-z_][\w]*["']?"#,
    )
}

fn regex_call_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r#"(?i)\b(regexp_extract|regexp_replace|regexp_substr|regexp_contains|rlike|regexp_like)\b\s*(?:\([^)]*\))?"#,
    )
}

fn normalization_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)\b(?:lower|upper)\s*\(\s*trim\s*\([^)]*\)\s*\)"#)
}

fn window_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?is)\bover\s*\((?P<body>[^)]*)\)"#)
}

fn join_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?is)\b(?:(left|right|full|inner|cross)\s+)?join\b"#)
}

fn is_leading_derived_comma_fp(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let Some(from_idx) = from_clause_tables_start(&lower) else {
        return false;
    };
    let tail = lower.get(from_idx..).unwrap_or_default().trim_start();
    if !tail.starts_with('(') {
        return false;
    }
    let mut depth = 0usize;
    for (byte_idx, ch) in tail.char_indices() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                let after = tail.get(byte_idx + 1..).unwrap_or_default().trim_start();
                return after.starts_with(',');
            }
        }
    }
    false
}

fn on_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?is)\bon\b\s*(?P<predicate>.*)"#)
}

fn equality_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)[\w.)"]+\s*=\s*[\w("]"#)
}

fn function_on_join_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r#"(?i)\bon\s+[^=]*(?:lower|upper|trim|ltrim|rtrim|cast|date|date_trunc|to_char|to_varchar)\s*\([^=]+=[^=]*(?:lower|upper|trim|ltrim|rtrim|cast|date|date_trunc|to_char|to_varchar|\)|\w+\s*\()|=\s*(?:lower|upper|trim|ltrim|rtrim|cast|date|date_trunc|to_char|to_varchar)\s*\("#,
    )
}

fn non_sargable_predicate_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r#"(?i)\bwhere\b[^;\n]*(?:timestamp_trunc|datetime|to_date)\s*\(\s*[^)]*(?:block_time|event_time|created_at|updated_at|event_date|ingested_at|_partitiontime|_partitiondate|partition_date|block_date|block_timestamp|block_number|evt_block_time|evt_block_number|evt_block_date|block_day)"#,
    )
}

fn extract_unions_without_all(
    text: &str,
    line_index: &costguard_diagnostics::LineIndex,
) -> Vec<ExpressionFeature> {
    union_keyword_regex()
        .find_iter(text)
        .filter(|matched| !is_union_all(text, matched.end()))
        .map(|matched| ExpressionFeature {
            span: line_index.span(matched.start(), matched.end()),
            key: "union".into(),
            text: matched.as_str().to_string(),
        })
        .collect()
}

fn is_union_all(text: &str, after_union: usize) -> bool {
    text[after_union..]
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("all")
}

fn union_keyword_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)\bunion\b"#)
}

fn count_distinct_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)\bcount\s*\(\s*distinct\b"#)
}

fn wildcard_table_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)`[^`]+\*`|from\s+[\w.]+\*|\bevents_\*"#)
}

fn cte_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?is)(?:with|,)\s+([a-z_][\w]*)\s+as\s*\("#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::LineIndex;

    #[test]
    fn extracts_select_star_and_window() {
        let text = "select *, row_number() over () as rn from a cross join b";
        let index = LineIndex::new(text);
        let doc = analyze_sql(
            PathBuf::from("x.sql"),
            text,
            Platform::Generic,
            &index,
            None,
            true,
        );
        assert_eq!(doc.features.select_stars.len(), 1);
        assert_eq!(doc.features.window_functions.len(), 1);
        assert_eq!(doc.features.joins.len(), 1);
    }

    #[test]
    fn normalize_strips_comments_and_try_cast() {
        let sql = "-- header\nSELECT TRY_CAST(x AS bigint) FROM t";
        let normalized = normalize_for_parse(sql, Platform::Trino);
        assert!(!normalized.contains("-- header"));
        assert!(normalized.contains("CAST("));
        assert!(!normalized.to_ascii_lowercase().contains("try_cast"));
    }

    #[test]
    fn compiled_fallback_uses_raw_when_compiled_unparseable() {
        let raw = "select id from stg";
        let compiled = "SELECT id FROM stg WHERE TRY_INVALID_SYNTAX(!!!";
        let index = LineIndex::new(raw);
        let doc = analyze_sql(
            PathBuf::from("x.sql"),
            raw,
            Platform::Trino,
            &index,
            Some(compiled),
            true,
        );
        assert!(!doc.parsed_compiled);
        assert!(doc.parsed_raw);
        assert!(doc.parsed);
        assert_eq!(doc.parse_input, ParseInput::CompiledWithRawFallback);
    }

    #[test]
    fn trino_try_cast_normalizes_to_parseable_sql() {
        let sql = "SELECT TRY_CAST(block_time AS timestamp) FROM dex.trades";
        let normalized = normalize_for_parse(sql, Platform::Trino);
        let dialect = Platform::Trino.sqlparser_dialect();
        assert!(Parser::parse_sql(dialect.as_ref(), &normalized).is_ok());
    }

    #[test]
    fn parses_largest_statement_from_multi_statement_compiled() {
        let sql = "SET session x = '1'; SELECT id FROM t WHERE block_time >= date_sub(current_date, interval '3' day);";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn trino_parses_underscore_identifiers_in_compiled_sql() {
        let sql = "SELECT current_timestamp AS _updated_at, s._dstChainId FROM s";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn trino_parses_json_query_omit_quotes() {
        let sql = "SELECT try_cast(json_query(x, 'lax $.a' omit quotes) as boolean) FROM t";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn trino_parses_cast_array_type_syntax() {
        let sql = "SELECT * FROM t CROSS JOIN UNNEST(CAST(JSON_EXTRACT(j, '$.s') AS ARRAY(VARCHAR))) AS u(strategy)";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn trino_parses_row_map_angle_type_syntax() {
        let sql = "SELECT CAST(NULL AS ROW(output map<varchar, JSON>)) FROM t";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn trino_parses_asof_left_join() {
        let sql = "SELECT hs.hour FROM hour_spine hs ASOF LEFT JOIN sparse_ohlcv s ON s.market_id = hs.market_id";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn plain_comma_join_is_flagged() {
        let text = "select a.id, b.id from stg_a a, stg_b b";
        let index = LineIndex::new(text);
        let doc = analyze_sql(
            PathBuf::from("x.sql"),
            text,
            Platform::Generic,
            &index,
            None,
            true,
        );
        assert!(
            doc.features
                .joins
                .iter()
                .any(|join| join.kind == JoinKind::Comma),
            "expected comma join; parsed_raw={} joins={:?}",
            doc.parsed_raw,
            doc.features.joins
        );
    }

    #[test]
    fn compiled_ast_fallback_uses_compiled_sql_when_raw_unparseable() {
        let raw = "{% set cols = ['id'] %} select {{ cols | join(', ') }} from t where date_trunc('day', block_time) >= current_date";
        let compiled = "select id from t where date_trunc('day', block_time) >= current_date";
        let index = LineIndex::new(raw);
        let doc = analyze_sql(
            PathBuf::from("models/marts/jinja.sql"),
            raw,
            Platform::Trino,
            &index,
            Some(compiled),
            true,
        );
        assert!(doc.parsed_compiled);
        assert!(!doc.parsed_raw);
        assert!(doc.feature_extraction_used_ast);
        assert!(doc.features.non_sargable_predicates.is_empty());
    }

    #[test]
    fn dbt_comma_join_with_refs_is_flagged() {
        let text = "select a.id, b.id\nfrom {{ ref('stg_a') }} a, {{ ref('stg_b') }} b\n";
        let index = LineIndex::new(text);
        let doc = analyze_sql(
            PathBuf::from("x.sql"),
            text,
            Platform::Generic,
            &index,
            None,
            true,
        );
        assert!(
            doc.features
                .joins
                .iter()
                .any(|join| join.kind == JoinKind::Comma),
            "expected comma join; parsed_raw={} joins={:?}",
            doc.parsed_raw,
            doc.features.joins
        );
    }

    #[test]
    fn cross_join_unnest_is_not_flagged() {
        let text = "SELECT t.x FROM arr CROSS JOIN UNNEST(arr) AS t(x)";
        let index = LineIndex::new(text);
        let doc = analyze_sql(
            PathBuf::from("x.sql"),
            text,
            Platform::Trino,
            &index,
            None,
            true,
        );
        assert!(!doc
            .features
            .joins
            .iter()
            .any(|join| matches!(join.kind, JoinKind::Cross | JoinKind::Comma)));
    }

    #[test]
    fn string_literal_cross_join_is_not_flagged_when_sql_parses() {
        let text = "select 'cross join in string' as note from t";
        let index = LineIndex::new(text);
        let doc = analyze_sql(
            PathBuf::from("x.sql"),
            text,
            Platform::Generic,
            &index,
            None,
            true,
        );
        assert!(!doc
            .features
            .joins
            .iter()
            .any(|join| matches!(join.kind, JoinKind::Cross | JoinKind::Comma)));
    }

    #[test]
    fn trino_parses_cross_join_unnest() {
        let sql = "SELECT time.time AS day FROM time_seq CROSS JOIN unnest(time) AS time(time)";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn trino_parses_wrapped_quoted_table() {
        let sql = r#"SELECT id FROM ("delta_prod"."across_v2_arbitrum"."arbitrum_spokepool_evt_fundsdeposited") d"#;
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }

    #[test]
    fn trino_parses_join_alias_without_as() {
        let sql = "SELECT b.epoch FROM base b LEFT JOIN base start ON start.epoch = b.epoch";
        assert!(try_parse_compiled_sql(sql, Platform::Trino));
    }
}

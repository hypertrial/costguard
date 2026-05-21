use costguard_dbt::{extract_sql_features, DbtSqlFeatures};
use costguard_diagnostics::Span;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SqlDialect {
    #[default]
    Generic,
    Snowflake,
    BigQuery,
    Databricks,
    Redshift,
    Postgres,
    DuckDB,
}

impl std::str::FromStr for SqlDialect {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "generic" => Ok(Self::Generic),
            "snowflake" => Ok(Self::Snowflake),
            "bigquery" => Ok(Self::BigQuery),
            "databricks" => Ok(Self::Databricks),
            "redshift" => Ok(Self::Redshift),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "duckdb" => Ok(Self::DuckDB),
            other => Err(format!("unknown SQL dialect '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SqlDocument {
    pub path: PathBuf,
    pub raw_sql: String,
    pub dialect: SqlDialect,
    pub parsed: bool,
    pub features: SqlFeatures,
    pub dbt: DbtSqlFeatures,
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
    pub function_on_both_sides: bool,
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
    dialect: SqlDialect,
    line_index: &costguard_diagnostics::LineIndex,
) -> SqlDocument {
    let sanitized = strip_jinja(text);
    let parsed = Parser::parse_sql(&GenericDialect {}, &sanitized).is_ok();
    let dbt = extract_sql_features(text);
    let features = extract_features(text, line_index);
    SqlDocument {
        path,
        raw_sql: text.to_string(),
        dialect,
        parsed,
        features,
        dbt,
    }
}

pub fn strip_jinja(text: &str) -> String {
    let without_blocks = jinja_block_regex().replace_all(text, " ");
    jinja_expr_regex()
        .replace_all(&without_blocks, " __jinja__ ")
        .to_string()
}

fn extract_features(text: &str, line_index: &costguard_diagnostics::LineIndex) -> SqlFeatures {
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
    features.joins = extract_joins(text, line_index);
    features.ctes = extract_ctes(text, line_index);
    features.cte_references = extract_cte_references(text, line_index, &features.ctes);
    features
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

fn extract_joins(text: &str, line_index: &costguard_diagnostics::LineIndex) -> Vec<JoinFeature> {
    let mut joins = Vec::new();
    for capture in join_regex().captures_iter(text) {
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
        let after_start = matched.end();
        let after_end = find_join_clause_end(&text[after_start..])
            .map(|offset| after_start + offset)
            .unwrap_or(text.len());
        let after = &text[after_start..after_end];
        let predicate = extract_on_predicate(after);
        let predicate_lower = predicate
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        joins.push(JoinFeature {
            span: line_index.span(matched.start(), after_end),
            kind,
            has_equality: has_equality_predicate(&predicate_lower),
            function_on_both_sides: function_on_both_sides(&predicate_lower),
            predicate,
        });
    }

    joins.extend(
        comma_join_regex()
            .find_iter(text)
            .filter(|matched| !matched.as_str().to_ascii_lowercase().contains("source("))
            .map(|matched| JoinFeature {
                span: line_index.span(matched.start(), matched.end()),
                kind: JoinKind::Comma,
                predicate: None,
                has_equality: false,
                function_on_both_sides: false,
            }),
    );
    joins
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

fn function_on_both_sides(predicate: &str) -> bool {
    function_both_sides_regex().is_match(predicate)
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
            let regex = Regex::new(&format!(r#"(?i)\b{}\b"#, regex::escape(&cte.name)))
                .expect("valid cte regex");
            regex
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

fn jinja_block_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?s)\{[%#].*?[%#]\}"#)
}

fn jinja_expr_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?s)\{\{.*?\}\}"#)
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

fn comma_join_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?is)\bfrom\s+[^;\n]+,\s*[^;\n]+"#)
}

fn on_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?is)\bon\b\s*(?P<predicate>.*)"#)
}

fn equality_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)[\w.)"]+\s*=\s*[\w("]"#)
}

fn function_both_sides_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r#"(?i)\bon\s+([a-z_][\w]*)\s*\([^=]+=\s*([a-z_][\w]*)\s*\("#,
    )
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
        let doc = analyze_sql(PathBuf::from("x.sql"), text, SqlDialect::Generic, &index);
        assert_eq!(doc.features.select_stars.len(), 1);
        assert_eq!(doc.features.window_functions.len(), 1);
        assert_eq!(doc.features.joins.len(), 1);
    }
}

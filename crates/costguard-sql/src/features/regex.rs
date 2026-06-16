use costguard_diagnostics::LineIndex;
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::{CteFeature, ExpressionFeature, JoinFeature, JoinKind, SqlFeatures, WindowFeature};

use super::comma_join;
use super::join_heuristics::{has_equality_predicate, is_date_spine_table};

fn leading_wildcard_like_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?i)\b(?:where|on)\b[^;\n]*\blike\s+'[%_][^']*'"#)
}

fn correlated_subquery_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        "(?i)\\b(?:where|on)\\b[^;\\n]*(?:exists\\s*\\(|\\(\\s*select\\b)[^;\\n]*\\b\\w+\\.\\w+\\s*=\\s*\\w+\\.\\w+",
    )
}

fn scalar_subquery_select_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, "(?i)\\bselect\\b[^;\\n]*?,\\s*\\(\\s*select\\b")
}

fn not_in_subquery_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, "(?i)\\bnot\\s+in\\s*\\(\\s*select\\b")
}

fn row_explosion_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        "(?i)\\b(?:lateral\\s+)?(?:flatten|unnest)\\s*\\(|\\bcross\\s+join\\s+unnest\\b",
    )
}

fn recursive_cte_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, "(?i)\\bwith\\s+recursive\\b")
}

fn extract_or_partition_predicates(text: &str, line_index: &LineIndex) -> Vec<ExpressionFeature> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\bwhere\b[^;\n]*\b(?:block_time|event_date|_partitiontime|_partitiondate|partition_date|evt_block_time|block_date)\b[^;\n]*\bor\b[^;\n]*\b(?:block_time|event_date|_partitiontime|_partitiondate|partition_date|evt_block_time|block_date)\b"#,
        )
        .expect("or partition regex")
    });
    regex
        .find_iter(text)
        .map(|matched| ExpressionFeature {
            span: line_index.span(matched.start(), matched.end()),
            key: "or partition predicate".into(),
            text: matched.as_str().to_string(),
        })
        .collect()
}

pub(crate) fn extract_features(
    text: &str,
    comma_text: &str,
    line_index: &LineIndex,
    parsed_raw: bool,
) -> SqlFeatures {
    // ponytail: only expensive-expression regexes see masked text; extend to all regex
    // features if comment FPs surface elsewhere.
    let code_text = mask_comments(text);
    let mut features = SqlFeatures::default();
    features.select_stars = matches_as_features(text, line_index, select_star_regex(), normalize);
    features.order_by_clauses =
        matches_as_features(text, line_index, order_by_regex(), |_| "order by".into());
    features.distincts = matches_as_features(text, line_index, distinct_regex(), |_| {
        "select distinct".into()
    });
    features.json_extractions =
        matches_as_features(&code_text, line_index, json_regex(), normalize_json_key);
    features.regex_calls =
        matches_as_features(&code_text, line_index, regex_call_regex(), normalize);
    features.normalization_calls =
        matches_as_features(&code_text, line_index, normalization_regex(), normalize);
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
    features.leading_wildcard_likes =
        matches_as_features(text, line_index, leading_wildcard_like_regex(), normalize);
    features.or_partition_predicates = extract_or_partition_predicates(text, line_index);
    features.correlated_subqueries =
        matches_as_features(text, line_index, correlated_subquery_regex(), |_| {
            "correlated subquery".into()
        });
    features.scalar_subqueries_in_select =
        matches_as_features(text, line_index, scalar_subquery_select_regex(), |_| {
            "scalar subquery".into()
        });
    features.not_in_subqueries =
        matches_as_features(text, line_index, not_in_subquery_regex(), |_| {
            "not in subquery".into()
        });
    features.row_explosions = matches_as_features(text, line_index, row_explosion_regex(), |_| {
        "row explosion".into()
    });
    features.recursive_ctes = matches_as_features(text, line_index, recursive_cte_regex(), |_| {
        "with recursive".into()
    });
    features
}

fn extract_wildcard_table_scans(text: &str, line_index: &LineIndex) -> Vec<ExpressionFeature> {
    if text.to_ascii_lowercase().contains("_table_suffix") {
        return Vec::new();
    }
    matches_as_features(text, line_index, wildcard_table_regex(), normalize)
}

fn matches_as_features<F>(
    text: &str,
    line_index: &LineIndex,
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

fn extract_windows(text: &str, line_index: &LineIndex) -> Vec<WindowFeature> {
    window_regex()
        .captures_iter(text)
        .filter_map(|capture| {
            let matched = capture.get(0)?;
            let body = capture.name("body")?.as_str();
            Some(WindowFeature {
                span: line_index.span(matched.start(), matched.end()),
                text: matched.as_str().to_string(),
                has_partition_by: body.to_ascii_lowercase().contains("partition by"),
                unbounded_frame: body.to_ascii_lowercase().contains("unbounded preceding")
                    && body.to_ascii_lowercase().contains("unbounded following"),
            })
        })
        .collect()
}

fn extract_joins(
    text: &str,
    comma_text: &str,
    line_index: &LineIndex,
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
            pattern_matching: predicate_matches_pattern(&predicate_lower),
            cross_catalog: cross_catalog_in_predicate(&predicate_lower),
            right_relation: None,
            equality_keys: Vec::new(),
        });
    }

    joins.extend(comma_join::extract_comma_joins(comma_text, line_index));
    annotate_cross_catalog_joins(&mut joins, join_text);
    joins
}

fn predicate_matches_pattern(predicate: &str) -> bool {
    let lower = predicate.to_ascii_lowercase();
    lower.contains(" like ")
        || lower.contains(" rlike ")
        || lower.contains(" regexp_like(")
        || lower.contains(" regexp ")
        || lower.contains(" similar to ")
}

fn cross_catalog_in_predicate(_predicate: &str) -> bool {
    false
}

fn annotate_cross_catalog_joins(joins: &mut [JoinFeature], text: &str) {
    let lower = text.to_ascii_lowercase();
    for join in joins.iter_mut() {
        if join.cross_catalog {
            continue;
        }
        let snippet = text
            .get(join.span.byte_start..join.span.byte_end.min(text.len()))
            .unwrap_or_default()
            .to_ascii_lowercase();
        join.cross_catalog = cross_catalog_snippet(&snippet) || cross_catalog_snippet(&lower);
    }
}

fn cross_catalog_snippet(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| {
        Regex::new(
            r"(?i)([\w-]+)\.([\w-]+)\.([\w`\-]+)[\s\S]{0,120}?\bjoin\b[\s\S]{0,120}?([\w-]+)\.([\w-]+)\.([\w`\-]+)",
        )
        .expect("cross catalog regex")
    });
    regex.captures(text).is_some_and(|caps| {
        caps.get(1)
            .zip(caps.get(4))
            .is_some_and(|(left, right)| left.as_str() != right.as_str())
    })
}

fn is_exempt_cross_join_suffix(after: &str) -> bool {
    let lower = after.trim_start().to_ascii_lowercase();
    if lower.starts_with("unnest(")
        || lower.starts_with("unnest (")
        || lower.starts_with("table(")
        || lower.starts_with("table (")
    {
        return true;
    }
    cross_join_table_name(after).is_some_and(|name| is_date_spine_table(&name))
}

fn cross_join_table_name(after: &str) -> Option<String> {
    let trimmed = after.trim_start();
    let mut end = trimmed.len();
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_whitespace() || ch == '(' {
            end = idx;
            break;
        }
    }
    let name = trimmed.get(..end)?.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

fn mask_comments(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut block_depth = 0usize;
    let mut in_jinja = false;

    while let Some(ch) = chars.next() {
        if in_jinja {
            if ch == '#' && chars.peek() == Some(&'}') {
                chars.next();
                output.push(' ');
                output.push(' ');
                in_jinja = false;
            } else {
                output.push(if ch == '\n' { '\n' } else { ' ' });
            }
            continue;
        }

        if block_depth > 0 {
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_depth += 1;
                output.push(' ');
                output.push(' ');
                continue;
            }
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_depth -= 1;
                output.push(' ');
                output.push(' ');
            } else {
                output.push(if ch == '\n' { '\n' } else { ' ' });
            }
            continue;
        }

        if !in_single && !in_double && ch == '{' && chars.peek() == Some(&'#') {
            chars.next();
            output.push(' ');
            output.push(' ');
            in_jinja = true;
            continue;
        }

        if !in_single && !in_double && ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block_depth = 1;
            output.push(' ');
            output.push(' ');
            continue;
        }

        if !in_single && !in_double && ch == '-' && chars.peek() == Some(&'-') {
            chars.next();
            output.push(' ');
            output.push(' ');
            for next in chars.by_ref() {
                if next == '\n' {
                    output.push('\n');
                    break;
                }
                output.push(' ');
            }
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

    output
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

fn function_on_join_key(predicate: &str) -> bool {
    let lower = predicate.to_ascii_lowercase();
    if is_symmetric_normalization_predicate(&lower) || is_time_bucket_join_predicate(&lower) {
        return false;
    }
    let parts: Vec<&str> = lower.split(" and ").collect();
    if parts.is_empty() {
        return function_on_join_key_regex().is_match(predicate);
    }
    parts.iter().any(|part| {
        let part = part.trim();
        !is_coalesce_null_safe_join_predicate(part) && function_on_join_key_regex().is_match(part)
    })
}

fn coalesce_null_safe_join_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r"(?i)coalesce\s*\(\s*(?:\w+\.)?(\w+)\s*,\s*(?:\w+\.)?(\w+)\s*\)\s*=\s*(?:\w+\.)?(\w+)\b",
    )
}

fn is_coalesce_null_safe_join_predicate(predicate: &str) -> bool {
    let Some(captures) = coalesce_null_safe_join_re().captures(predicate) else {
        return false;
    };
    let (Some(left), Some(right), Some(key)) = (
        captures.get(1).map(|m| m.as_str()),
        captures.get(2).map(|m| m.as_str()),
        captures.get(3).map(|m| m.as_str()),
    ) else {
        return false;
    };
    left == right && left == key
}

fn is_time_bucket_join_predicate(predicate: &str) -> bool {
    let lower = predicate.to_ascii_lowercase();
    if !lower.contains('=') {
        return false;
    }
    time_bucket_column_re().is_match(&lower) && date_trunc_join_re().is_match(&lower)
}

fn is_symmetric_normalization_predicate(predicate: &str) -> bool {
    for func in [
        "lower",
        "upper",
        "trim",
        "ltrim",
        "rtrim",
        "date_trunc",
        "coalesce",
        "cast",
    ] {
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
    line_index: &LineIndex,
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

fn on_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&RE, r#"(?is)\bon\b\s*(?P<predicate>.*)"#)
}

fn function_on_join_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r#"(?i)\bon\s+[^=]*(?:lower|upper|trim|ltrim|rtrim|cast|date|date_trunc|to_char|to_varchar)\s*\([^=]+=[^=]*(?:lower|upper|trim|ltrim|rtrim|cast|date|date_trunc|to_char|to_varchar|\)|\w+\s*\()|=\s*(?:lower|upper|trim|ltrim|rtrim|cast|date|date_trunc|to_char|to_varchar)\s*\("#,
    )
}

static TIME_BUCKET_COLUMN_RE: OnceLock<Regex> = OnceLock::new();
static DATE_TRUNC_JOIN_RE: OnceLock<Regex> = OnceLock::new();

fn time_bucket_column_re() -> &'static Regex {
    TIME_BUCKET_COLUMN_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(?:minute|hour|day|date|week|month|block_date|block_day|evt_block_date)\b"#,
        )
        .expect("time bucket column regex")
    })
}

fn date_trunc_join_re() -> &'static Regex {
    DATE_TRUNC_JOIN_RE.get_or_init(|| {
        Regex::new(r#"(?i)\b(?:date_trunc|timestamp_trunc|datetime|date)\s*\("#)
            .expect("date trunc join regex")
    })
}

fn non_sargable_predicate_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(
        &RE,
        r#"(?i)\bwhere\b[^;\n]*(?:timestamp_trunc|datetime|to_date)\s*\(\s*[^)]*(?:block_time|event_time|created_at|updated_at|event_date|ingested_at|_partitiontime|_partitiondate|partition_date|block_date|block_timestamp|block_number|evt_block_time|evt_block_number|evt_block_date|block_day)"#,
    )
}

fn extract_unions_without_all(text: &str, line_index: &LineIndex) -> Vec<ExpressionFeature> {
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

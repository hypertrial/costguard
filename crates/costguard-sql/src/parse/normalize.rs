use crate::Platform;
use regex::Regex;
use std::sync::OnceLock;

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

pub(crate) fn apply_trino_relaxed_rewrites(text: &str) -> String {
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

pub(crate) fn largest_query_statement(text: &str) -> Option<String> {
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

fn cached_regex(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

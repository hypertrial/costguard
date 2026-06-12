use crate::{DbtConfig, DbtRef, DbtSourceRef, DbtSqlFeatures};
use regex::Regex;
use std::sync::OnceLock;

pub fn extract_sql_features(text: &str) -> DbtSqlFeatures {
    let mut features = DbtSqlFeatures::default();

    for capture in ref_regex().captures_iter(text) {
        let (package, name) = match capture.get(2) {
            Some(name) => (Some(capture[1].to_string()), name.as_str().to_string()),
            None => (None, capture[1].to_string()),
        };
        features.refs.push(DbtRef { package, name });
    }

    for capture in source_regex().captures_iter(text) {
        features.sources.push(DbtSourceRef {
            source_name: capture[1].to_string(),
            table_name: capture[2].to_string(),
        });
    }

    apply_inline_configs(text, &mut features.config);

    features.uses_is_incremental = text.contains("is_incremental()");
    features.incremental_block = extract_incremental_block(text);
    features
}

pub fn extract_incremental_block(text: &str) -> Option<String> {
    incremental_block_regex()
        .captures(text)
        .and_then(|capture| capture.get(1))
        .map(|matched| matched.as_str().trim().to_string())
        .filter(|block| !block.is_empty())
}

fn incremental_block_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?is)\{%\s*(?:if|elif)\s+is_incremental\s*\(\)\s*%\}(.*?)\{%\s*endif\s*%\}")
            .expect("valid incremental regex")
    })
}

fn apply_inline_configs(text: &str, config: &mut DbtConfig) {
    for body in config_call_bodies(text) {
        for item in split_top_level_commas(body) {
            let Some((key, value)) = split_top_level_assignment(item) else {
                continue;
            };
            match key.trim() {
                "materialized" => {
                    if let Some(value) = quoted_string(value.trim()) {
                        config.materialized = Some(value);
                    }
                }
                "unique_key" => {
                    if let Some(value) = parse_unique_key_literal(value.trim()) {
                        config.unique_key = Some(value);
                    }
                }
                "incremental_strategy" => {
                    if let Some(value) = quoted_string(value.trim()) {
                        config.incremental_strategy = Some(value);
                    }
                }
                "partition_by" => {
                    if let Some(value) = parse_inline_config_value(value.trim()) {
                        config.partition_by = Some(value);
                    }
                }
                "cluster_by" => {
                    if let Some(value) = parse_inline_config_value(value.trim()) {
                        config.cluster_by = Some(value);
                    }
                }
                "full_refresh" => {
                    if let Some(value) = parse_bool_literal(value.trim()) {
                        config.full_refresh = Some(value);
                    }
                }
                "on_schema_change" => {
                    if let Some(value) = quoted_string(value.trim()) {
                        config.on_schema_change = Some(value);
                    }
                }
                _ => {}
            }
        }
    }
}

fn config_call_bodies(text: &str) -> Vec<&str> {
    let lower = text.to_ascii_lowercase();
    let mut bodies = Vec::new();
    let mut index = 0usize;
    while index < text.len() {
        if lower[index..].starts_with("config") && is_identifier_boundary(text, index) {
            let mut open = index + "config".len();
            while open < text.len() {
                let ch = text[open..].chars().next().unwrap_or_default();
                if !ch.is_whitespace() {
                    break;
                }
                open += ch.len_utf8();
            }
            if text[open..].starts_with('(') {
                if let Some(close) = balanced_config_close(text, open) {
                    bodies.push(&text[open + 1..close]);
                    index = close + 1;
                    continue;
                }
            }
        }
        let ch = text[index..].chars().next().unwrap_or_default();
        if ch == '\0' {
            break;
        }
        index += ch.len_utf8();
    }
    bodies
}

fn is_identifier_boundary(text: &str, index: usize) -> bool {
    text[..index]
        .chars()
        .next_back()
        .is_none_or(|ch| !is_identifier_char(ch))
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn balanced_config_close(text: &str, open: usize) -> Option<usize> {
    let mut paren_depth = 1usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut index = open + 1;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        if let Some(quote_ch) = quote {
            if ch == '\\' {
                index += ch.len_utf8();
                if index < text.len() {
                    index += text[index..].chars().next()?.len_utf8();
                }
                continue;
            }
            if ch == quote_ch {
                let next = index + ch.len_utf8();
                if text[next..].starts_with(quote_ch) {
                    index = next + ch.len_utf8();
                    continue;
                }
                quote = None;
            }
            index += ch.len_utf8();
            continue;
        }

        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' if bracket_depth == 0 && brace_depth == 0 => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 {
                    return Some(index);
                }
            }
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
        index += ch.len_utf8();
    }
    None
}

fn split_top_level_commas(text: &str) -> Vec<&str> {
    split_top_level(text, ',')
}

pub(crate) fn split_top_level_assignment(text: &str) -> Option<(&str, &str)> {
    let parts = split_top_level(text, '=');
    match parts.as_slice() {
        [key, value] => Some((*key, *value)),
        _ => None,
    }
}

fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut index = 0usize;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap_or_default();
        if let Some(quote_ch) = quote {
            if ch == '\\' {
                index += ch.len_utf8();
                if index < text.len() {
                    index += text[index..].chars().next().unwrap_or_default().len_utf8();
                }
                continue;
            }
            if ch == quote_ch {
                quote = None;
            }
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            _ if ch == delimiter && paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(text[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
        index += ch.len_utf8();
    }
    parts.push(text[start..].trim());
    parts
}

fn parse_unique_key_literal(value: &str) -> Option<String> {
    if let Some(value) = quoted_string(value) {
        return Some(value);
    }
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .or_else(|| {
            value
                .strip_prefix('(')
                .and_then(|value| value.strip_suffix(')'))
        })?;
    let parts = split_top_level_commas(inner)
        .into_iter()
        .map(str::trim)
        .map(quoted_string)
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join(","))
}

fn quoted_string(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    if !value.ends_with(quote) || value.len() < quote.len_utf8() * 2 {
        return None;
    }
    let inner = &value[quote.len_utf8()..value.len() - quote.len_utf8()];
    Some(inner.to_string())
}

fn parse_bool_literal(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_inline_config_value(value: &str) -> Option<String> {
    quoted_string(value).or_else(|| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn ref_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"ref\s*\(\s*['"]([^'"]+)['"]\s*(?:,\s*['"]([^'"]+)['"]\s*)?\)"#)
            .expect("valid regex")
    })
}

fn source_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"source\s*\(\s*['"]([^'"]+)['"]\s*,\s*['"]([^'"]+)['"]\s*\)"#)
            .expect("valid regex")
    })
}

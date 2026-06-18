use costguard_diagnostics::LineIndex;

use super::join_heuristics::is_date_spine_table;
use crate::{JoinFeature, JoinKind};

pub(crate) fn from_clause_tables_start(lower: &str) -> Option<usize> {
    from_clause_tables_start_masked(&mask_line_and_jinja_comments(lower))
}

fn from_clause_tables_start_masked(lower: &str) -> Option<usize> {
    let patterns = [" from ", "\nfrom ", "\r\nfrom ", "\tfrom "];
    let mut depth = 0usize;
    let mut last_at_depth_zero: Option<usize> = None;
    let mut i = 0usize;
    while i < lower.len() {
        let ch = lower[i..].chars().next().unwrap();
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
        } else if depth == 0 {
            if lower[i..].starts_with("from ") {
                last_at_depth_zero = Some(i + 5);
            } else {
                for pattern in patterns {
                    if lower[i..].starts_with(pattern) {
                        last_at_depth_zero = Some(i + pattern.len());
                        break;
                    }
                }
            }
        }
        i += ch.len_utf8();
    }
    last_at_depth_zero
}

fn mask_line_and_jinja_comments(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < text.len() {
        if lower[i..].starts_with("--") {
            let start = i;
            while i < text.len() && text.as_bytes()[i] != b'\n' {
                i += 1;
            }
            output.push_str(&" ".repeat(i.saturating_sub(start)));
            if i < text.len() {
                output.push('\n');
                i += 1;
            }
            continue;
        }
        if lower[i..].starts_with("{#") {
            if let Some(rel) = lower[i..].find("#}") {
                let end = i + rel + 2;
                output.push_str(&" ".repeat(end.saturating_sub(i)));
                i = end;
                continue;
            }
        }
        let ch = text[i..].chars().next().expect("char");
        output.push(ch);
        i += ch.len_utf8();
    }
    output
}

fn at_from_table_list_boundary(lower: &str, index: usize) -> bool {
    [
        " where ",
        " group by ",
        " order by ",
        " having ",
        " limit ",
        "\nwhere ",
        "\ngroup by ",
        "\norder by ",
        "\nhaving ",
        "\nlimit ",
    ]
    .iter()
    .any(|boundary| lower[index..].starts_with(boundary))
}

fn at_explicit_join_boundary(lower: &str, index: usize) -> bool {
    [
        " inner join ",
        " left join ",
        " right join ",
        " full join ",
        " cross join ",
        " join ",
        "\ninner join ",
        "\nleft join ",
        "\nright join ",
        "\nfull join ",
        "\ncross join ",
        "\njoin ",
    ]
    .iter()
    .any(|boundary| lower[index..].starts_with(boundary))
}
pub(crate) fn extract_comma_joins(text: &str, line_index: &LineIndex) -> Vec<JoinFeature> {
    let masked = mask_line_and_jinja_comments(text);
    let lower = masked.to_ascii_lowercase();
    let Some(from_start) = from_clause_tables_start_masked(&lower) else {
        return Vec::new();
    };
    let tail = masked.get(from_start..).unwrap_or_default();
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
        if let Some(rel) = comma_join_right_relation(tail, comma_rel) {
            if is_date_spine_table(&rel) {
                return Vec::new();
            }
        }
        return vec![JoinFeature {
            span: line_index.span(span_start, span_end),
            kind: JoinKind::Comma,
            predicate: None,
            has_equality: false,
            function_on_join_key: false,
            pattern_matching: false,
            cross_catalog: false,
            right_relation: comma_join_right_relation(tail, comma_rel),
            equality_keys: Vec::new(),
            extracted_from_ast: false,
        }];
    }
    Vec::new()
}

fn find_top_level_comma_after_from(text: &str) -> Option<usize> {
    let masked = mask_line_and_jinja_comments(text);
    let lower = masked.to_ascii_lowercase();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut past_table_list = false;
    let mut saw_explicit_join = false;
    let mut i = 0usize;
    while i < masked.len() {
        if past_table_list {
            return None;
        }
        if paren_depth == 0 && bracket_depth == 0 {
            if at_explicit_join_boundary(&lower, i) {
                saw_explicit_join = true;
            }
            if at_from_table_list_boundary(&lower, i) {
                past_table_list = true;
            }
        }
        let ch = masked[i..].chars().next()?;
        if past_table_list {
            i += ch.len_utf8();
            continue;
        }
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 => {
                if saw_explicit_join {
                    return None;
                }
                let tail = masked[i + 1..].trim_start();
                if tail.chars().next().is_some_and(|first| {
                    first.is_ascii_alphabetic() || first == '(' || first == '_'
                }) {
                    return Some(i);
                }
                return None;
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn is_comma_join_false_positive(text: &str) -> bool {
    is_leading_derived_comma_fp(&format!("from{text}")) || looks_like_subquery_comma_fp(text)
}

fn looks_like_subquery_comma_fp(text: &str) -> bool {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('(') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let mut depth = 0usize;
    let mut select_before_comma = false;
    for (idx, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 1 {
                    return false;
                }
                depth = depth.saturating_sub(1);
            }
            ',' if depth == 1 => return select_before_comma,
            _ => {
                if depth == 1 && !select_before_comma && lower[idx..].starts_with("select") {
                    select_before_comma = true;
                }
            }
        }
    }
    false
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

fn comma_join_right_relation(text: &str, comma_rel: usize) -> Option<String> {
    let after = text.get(comma_rel + 1..)?.trim_start();
    let mut name = String::new();
    for ch in after.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch.to_ascii_lowercase());
        } else {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

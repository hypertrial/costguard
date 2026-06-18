pub fn is_date_spine_table(name: &str) -> bool {
    let normalized = name
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .to_ascii_lowercase();
    let name = normalized.rsplit('.').next().unwrap_or(&normalized);
    matches!(
        name,
        "check_date"
            | "date_spine"
            | "time_seq"
            | "calendar"
            | "dates"
            | "time_dimension"
            | "date_ranges"
            | "create_date_range"
            | "create_week_range"
            | "create_month_range"
            | "clean_month_range"
    )
}

pub fn has_equality_predicate(predicate: &str) -> bool {
    let lower = predicate.trim_start().to_ascii_lowercase();
    if lower.starts_with("using(") || lower.starts_with("using (") {
        return true;
    }

    let bytes = predicate.as_bytes();
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte != b'=' {
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|prev| bytes.get(prev)).copied();
        let next = bytes.get(idx + 1).copied();
        if matches!(prev, Some(b'<') | Some(b'>') | Some(b'!') | Some(b'='))
            || matches!(next, Some(b'>') | Some(b'='))
        {
            continue;
        }
        return true;
    }
    false
}

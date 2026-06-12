pub fn is_date_spine_table(name: &str) -> bool {
    matches!(
        name,
        "check_date"
            | "date_spine"
            | "time_seq"
            | "calendar"
            | "dates"
            | "time_dimension"
            | "date_ranges"
    )
}

pub fn has_equality_predicate(predicate: &str) -> bool {
    predicate.contains('=') && !predicate.contains(">=") && !predicate.contains("<=")
}

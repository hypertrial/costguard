use serde::Serialize;

/// Compact context-only issue for unchanged failures in PR mode.
#[derive(Debug, Clone, Serialize)]
pub struct ContextIssue {
    pub rule_id: String,
    pub path: String,
    pub message: String,
}

/// Full-project context report (nonblocking in PR mode).
#[derive(Debug, Clone, Serialize)]
pub struct ContextReport {
    pub counts: costguard_scanner::ScanCounts,
    pub sql_parse_total: usize,
    pub sql_parse_failures: usize,
    pub sql_parse_other_total: usize,
    pub sql_parse_other_failures: usize,
    pub skipped_count: usize,
    pub issues: Vec<ContextIssue>,
}

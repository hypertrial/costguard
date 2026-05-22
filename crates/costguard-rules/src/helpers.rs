use crate::registry::{RuleContext, Warehouse};
use costguard_diagnostics::{Diagnostic, Severity, Span};
use std::path::Path;

pub(crate) fn diagnostic(
    ctx: &RuleContext<'_>,
    rule_id: &str,
    severity: Severity,
    span: Option<Span>,
    message: &str,
) -> Diagnostic {
    Diagnostic::new(
        rule_id,
        severity,
        ctx.file.root_relative_path.clone(),
        span,
        message,
    )
    .with_warehouse(ctx.warehouse.to_string())
}

pub(crate) fn threshold(ctx: &RuleContext<'_>, rule_id: &str, default: usize) -> usize {
    ctx.overrides
        .get(rule_id)
        .and_then(|settings| settings.threshold)
        .unwrap_or(default)
}

pub(crate) fn is_downstream_model(path: &Path) -> bool {
    let path = normalized_path(path);
    path.contains("marts/")
        || path.contains("intermediate/")
        || path.contains("/int_")
        || path.contains("/fct_")
        || path.contains("/dim_")
}

pub(crate) fn is_staging_model(path: &Path) -> bool {
    let path = normalized_path(path);
    path.contains("staging/") || path.contains("/stg_")
}

pub(crate) fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

pub(crate) fn has_bounded_incremental_predicate(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if [
        "updated_at",
        "created_at",
        "event_date",
        "ingested_at",
        "_partitiontime",
        "_partitiondate",
        "partition_date",
        "block_time",
        "evt_block_time",
        "block_date",
        "block_timestamp",
        "block_number",
        "block_num",
        "evt_block_number",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return true;
    }

    if [" date_trunc(", " minute", " hour"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return true;
    }

    [
        "not in (select",
        "from {{ this }}",
        "> (select max(",
        "except select",
        "where id not in",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(crate) fn incremental_source_filter_deferred(
    text: &str,
    uses_is_incremental: bool,
    has_sources: bool,
    materialized: Option<&str>,
    incremental_block: Option<&str>,
) -> bool {
    if !uses_is_incremental || !has_sources || materialized != Some("incremental") {
        return false;
    }
    let predicate_scope = incremental_block.unwrap_or(text);
    if !has_bounded_incremental_predicate(predicate_scope) {
        return false;
    }
    source_read_lacks_local_partition_predicate(text)
}

fn source_read_lacks_local_partition_predicate(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut search_start = 0;
    while let Some(rel_idx) = lower[search_start..].find("source(") {
        let abs_idx = search_start + rel_idx;
        let before = &lower[..abs_idx];
        let select_start = before.rfind("select").unwrap_or(0);
        let after = &text[abs_idx..];
        let close = after
            .find("),")
            .or(after.find("\n),"))
            .unwrap_or(after.len().min(1200));
        let scope = &text[select_start..abs_idx + close];
        if source_scope_lacks_partition_predicate(scope) {
            return true;
        }
        search_start = abs_idx + 7;
    }
    false
}

fn source_scope_lacks_partition_predicate(scope: &str) -> bool {
    let lower = scope.to_ascii_lowercase();
    let Some(where_idx) = lower.rfind("where") else {
        return true;
    };
    !has_bounded_incremental_predicate(&lower[where_idx..])
}

pub(crate) fn incremental_predicate_suggestion(warehouse: Warehouse) -> &'static str {
    match warehouse {
        Warehouse::BigQuery => {
            "add a partition predicate such as _PARTITIONDATE or an event_date filter."
        }
        Warehouse::Snowflake => {
            "add an updated_at/event_date predicate to improve pruning on clustered or micro-partitioned data."
        }
        Warehouse::Databricks => "add a partition or date predicate to limit incremental reads.",
        _ => "add an updated_at, created_at, event_date, block_time, ingested_at, or partition predicate.",
    }
}

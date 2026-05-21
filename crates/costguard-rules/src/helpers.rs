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
    ]
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

pub(crate) fn incremental_predicate_suggestion(warehouse: Warehouse) -> &'static str {
    match warehouse {
        Warehouse::BigQuery => {
            "add a partition predicate such as _PARTITIONDATE or an event_date filter."
        }
        Warehouse::Snowflake => {
            "add an updated_at/event_date predicate to improve pruning on clustered or micro-partitioned data."
        }
        Warehouse::Databricks => "add a partition or date predicate to limit incremental reads.",
        _ => "add an updated_at, created_at, event_date, ingested_at, or partition predicate.",
    }
}

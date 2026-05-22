# SQLCOST021: BigQuery wildcard table scan without suffix bound

**Severity:** medium

**Platform:** BigQuery only

Detects wildcard tables such as `` `project.dataset.events_*` `` without a bounded `_TABLE_SUFFIX` filter.

Add a `_TABLE_SUFFIX` between filter or otherwise bound the wildcard scan.

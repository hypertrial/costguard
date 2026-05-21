# SQLCOST004: Incremental model without unique_key

Detects dbt incremental models without `config(unique_key=...)`.

Append-only models may be valid, but merge/update incremental models should define stable keys.

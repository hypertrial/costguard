# SQLCOST004: Incremental model without unique_key

Detects dbt incremental models without a configured `unique_key`.

Config can come from inline SQL `{{ config(...) }}`, schema/properties YAML (`models[].config.unique_key`), nested `dbt_project.yml` folder defaults, or a compiled manifest.

Append-only incrementals are exempt when `incremental_strategy: append` is set explicitly in SQL or YAML config.

Merge/update incremental models should define stable keys unless they intentionally use append strategy.

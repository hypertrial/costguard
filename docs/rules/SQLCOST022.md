# SQLCOST022: Python model collects warehouse data locally

**Severity:** medium

Detects Python dbt patterns such as `.collect()`, `.toPandas()`, `.to_pandas()`, or broad local dataframe conversion.

Keep transformations in the warehouse dataframe API instead of pulling large result sets into driver memory.

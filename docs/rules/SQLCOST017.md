# SQLCOST017: Schema YAML parse failure

**Severity:** low

Reports when a dbt schema YAML file failed to parse. Costguard continues with empty YAML metadata for that file.

Fix the YAML syntax so model config, tests, and column metadata are available to incremental and graph rules.

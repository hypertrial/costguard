# SQLCOST026: File skipped during scan

**Severity:** low

Reports when a SQL or dbt file exceeds the configured scan size limit and was not loaded.

Narrow scan paths, ignore generated files, or raise `[scan].max_file_bytes` in `costguard.toml` (default 5242880 bytes; set `0` to keep the built-in default).

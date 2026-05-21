# SQLCOST003: Repeated regex extraction or replacement

Detects repeated regex work such as `regexp_extract`, `regexp_replace`, `regexp_substr`, and `rlike`.

Prefer simpler string functions or upstream tokenization when possible.

# SQLCOST018: dbt_project.yml metadata issue

**Severity:** low

Reports when `dbt_project.yml` failed to parse or has an ambiguous `models:` block (multiple project keys without a matching project name).

Fix the project file so folder-level model config can be applied.

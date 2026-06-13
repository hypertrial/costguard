# Support policy

Costguard 2.x supports the latest stable release on Linux x86-64, macOS ARM64, macOS x86-64, and Windows x86-64. The server image supports Linux x86-64 with PostgreSQL 17.

Generic SQL, Snowflake, BigQuery, and Trino are production-supported. Databricks, Redshift, Postgres, and DuckDB are preview. Preview dialects receive best-effort fixes but may change in minor releases as parser coverage improves.

Open a GitHub issue with the Costguard version, warehouse, minimal SQL or dbt fixture, configuration, expected behavior, and actual output. Security reports must follow [SECURITY.md](SECURITY.md).

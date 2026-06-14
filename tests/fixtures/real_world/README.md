# Real-world dbt fixture snippets

Minimal dbt-style projects extracted and adapted from public repositories for offline
benchmarking and regression testing. These are not full repo clones.

| Subdirectory | Source repo | Pin commit | License |
| --- | --- | --- | --- |
| `jaffle_snippets/` | [dbt-labs/jaffle-shop](https://github.com/dbt-labs/jaffle-shop) | `7be2c5838dbdeca8e915d4e46db70e910753d7f6` | Apache-2.0 |
| `spellbook_snippets/` | [duneanalytics/spellbook](https://github.com/duneanalytics/spellbook) | `031a5053dd9608ce7e6b9f2d9b16dd9a2fbeba10` | Apache-2.0 |
| `data_infra_snippets/` | [cal-itp/data-infra](https://github.com/cal-itp/data-infra) | `5d9dd8849fa0a4830d33e6783a4d152f35fdc71f` | AGPL-3.0 |
| `snowflake_snippets/` | Adapted Snowflake-style dbt patterns | n/a | MIT |
| `manifest_graph/` | Adapted from Costguard fixtures + jaffle patterns | n/a | MIT |
| `multi_dbt/` | Multi-package dbt layout smoke fixture | n/a | MIT |

Run benchmarks:

```bash
python3 scripts/benchmark_external_repo.py --all-vendored
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/spellbook_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/data_infra_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/snowflake_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/manifest_graph
python3 scripts/benchmark_external_repo.py --fixture real_world/multi_dbt
```

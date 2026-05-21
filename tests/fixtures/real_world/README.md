# Real-world dbt fixture snippets

Minimal dbt-style projects extracted and adapted from public repositories for offline
benchmarking and regression testing. These are not full repo clones.

| Subdirectory | Source repo | Pin commit | License |
| --- | --- | --- | --- |
| `jaffle_snippets/` | [dbt-labs/jaffle-shop](https://github.com/dbt-labs/jaffle-shop) | `7be2c5838dbdeca8e915d4e46db70e910753d7f6` | Apache-2.0 |
| `spellbook_snippets/` | [duneanalytics/spellbook](https://github.com/duneanalytics/spellbook) | `60f3b3fded8bae7d55780e7f8e6b15b1249d16a6` | Apache-2.0 |
| `manifest_graph/` | Adapted from Costguard fixtures + jaffle patterns | n/a | MIT |

Run benchmarks:

```bash
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/spellbook_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/manifest_graph
```

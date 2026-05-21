# Dune Spellbook Stress Test Plan

Dune Spellbook (`duneanalytics/spellbook`) should be the first public-real stress test for `costguard`.

Primary repo: <https://github.com/duneanalytics/spellbook>

Why Spellbook:

- It is a real production-style dbt project, not a tutorial.
- It is public, large, active, macro-heavy, and structurally complex.
- It has `models/`, `sources/`, `dbt_macros/`, and multiple `dbt_subprojects/`.
- Its blockchain analytics domain creates realistic SQL and Jinja patterns across DEX, NFT, Solana, and token datasets.
- It should expose scanner gaps, dbt graph assumptions, noisy rules, and parser resilience issues quickly.

Initial command set:

```bash
git clone https://github.com/duneanalytics/spellbook.git
cd spellbook

costguard scan . --warehouse generic
costguard scan models --warehouse generic --format json > costguard-spellbook.json
costguard scan dbt_subprojects --warehouse generic
costguard scan . --warehouse generic --fail-on high
```

Later, if project-directory workflows need targeted checks:

```bash
costguard scan dbt_subprojects/dex --warehouse generic
costguard scan dbt_subprojects/nft --warehouse generic
costguard scan dbt_subprojects/solana --warehouse generic
costguard scan dbt_subprojects/tokens --warehouse generic
```

Metrics to capture:

| Metric | Why |
| --- | --- |
| Total files scanned | scanner correctness |
| SQL/Jinja parse failure rate | robustness |
| Diagnostics per rule | noisy-rule detection |
| High-severity false positives | MVP quality |
| Runtime | Rust performance value |
| Peak memory | enterprise-scale viability |
| Suppression needs | rule ergonomics |
| Crash count | parser resilience |

Benchmark tiers:

```text
tier_0_smoke:   dbt-labs/jaffle-shop
tier_1_real:    mattermost/mattermost-data-warehouse
tier_2_stress:  duneanalytics/spellbook
tier_3_breadth: selected repos from InfuseAI/awesome-public-dbt-projects
tier_4_scale:   synthetic 1k/5k/10k model generated repos
```

Use Spellbook as the primary public-real stress test before expanding to the broader public dbt corpus.

Secondary repos to add after Spellbook:

- Mattermost data warehouse: <https://github.com/mattermost/mattermost-data-warehouse/tree/master/transform/snowflake-dbt>
- Cal-ITP data infrastructure: <https://github.com/cal-itp/data-infra/>
- dbt Jaffle Shop smoke test: <https://github.com/dbt-labs/jaffle-shop>
- Public dbt corpus: <https://github.com/InfuseAI/awesome-public-dbt-projects>

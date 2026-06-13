# Glossary

Canonical terminology for Costguard documentation. Prefer these definitions in README, design docs, and benchmarks.

## Product terms

| Term | Definition |
| --- | --- |
| **PR check** | Primary product workflow: `costguard pr` scans changed files against the git base and fails on high-confidence, high-severity findings |
| **CLI engine** | Local `costguard` binary powering PR checks, scans, and CI |
| **Cost regression guardrail** | Local, dbt-aware static analysis that catches expensive SQL before it merges—not a FinOps billing or observability platform |
| **Blast radius** | Downstream models affected by a changed upstream model (PR summary enrichment) |

## Platform and configuration

| Term | Definition |
| --- | --- |
| **`warehouse`** | Preferred config/CLI key for SQL dialect selection (`Platform` enum) |
| **`dialect`** | Config/CLI alias for `warehouse`; ignored when `warehouse` is set |
| **`fail-on`** | Minimum diagnostic severity that causes exit code `1` |
| **`min-confidence`** | Optional confidence floor paired with `fail-on`; finding must meet both thresholds to fail |
| **Severity order** | `info` < `low` < `medium` < `high` < `critical` (aliases: `med`, `crit`) |

## Parse metrics

| Term | Definition |
| --- | --- |
| **Headline parse** | `sql_parse_failures` — compiled parse with stripped-raw fallback |
| **Compiled parse** | Parse attempt on manifest `compiled_code` after normalization |
| **`sql_parse_compiled_failures`** | Compiled-only failures; Spellbook gate requires `0` |
| **`sql_parse_other_*`** | Parse metrics for non-model SQL (macros, tests, etc.) |
| **Feature extraction mode** | AST-based shape features when SQL parses; regex fallback otherwise (`feature_extraction_used_ast`) |
| **FP registry** | Machine-readable false-positive contracts in `tests/benchmarks/fp_registry.toml` |
| **Max-count ceiling** | External baseline `max_diagnostics_by_rule` — rule counts may shrink, not grow |

## Benchmark layers (canonical)

Use this four-layer model in all docs. Legacy tier names map in the footnote column.

| Layer | What | How to run |
| --- | --- | --- |
| **1. Corpus** | Deterministic rule contracts | `cargo test -p costguard-core --test corpus` |
| **2. Vendored** | Offline real-world snippets | `python3 scripts/benchmark_external_repo.py --all-vendored` |
| **3. External** | Pinned public repo clones | `python3 scripts/benchmark_external_repo.py --repo spellbook` |
| **4. Synthetic scale** | Generated 1k/5k/10k model repos | `scripts/generate_synthetic_dbt.py` |

### Legacy tier mapping

| Legacy name | Maps to |
| --- | --- |
| README tier 0 (jaffle smoke) | External layer (`--repo jaffle-shop`) |
| README tier 1 (vendored) | Vendored layer |
| README tier 2 (spellbook stress) | External layer (`--repo spellbook`) |
| README tier 3 (synthetic) | Synthetic scale layer |
| Spellbook doc `tier_0_smoke` | External (jaffle-shop) |
| Spellbook doc `tier_2_stress` | External (spellbook) |
| Spellbook doc `tier_4_scale` | Synthetic scale layer |
| Calibration "three-tier" (corpus/vendored/external) | Layers 1–3 above |

## Suppressions

| Term | Definition |
| --- | --- |
| **`disable-next-line=RULE`** | Suppress a rule on the following line |
| **`allow cross-join`** | Document intentional cross join for SQLCOST012 |

Prefer SQL comment prefix `-- costguard: ...`; bare `costguard:` comments are also parsed.

## Documentation style

- Use imperative headings in reference pages
- Prefix script invocations with `python3`
- Show `--warehouse` on scan examples even when optional
- CI examples: prefer `--base origin/main` with footnote that CLI default is `main`

## Related

- [Benchmark tiers](contributing/benchmark-tiers.md)
- [Parse metrics](reference/parse-metrics.md)
- [Platforms](reference/platforms.md)

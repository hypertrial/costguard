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
| **Eval labels** | Frozen binary-classification dataset in `tests/benchmarks/eval_labels.toml`; seeded from corpus and fp_registry |
| **Classification metrics** | Precision, recall, F1, MCC, PR-AUC, ROC-AUC from `scripts/eval_metrics.py` |
| **LLM judge labels** | Committed second-rater verdicts in `tests/benchmarks/llm_judge_labels.jsonl`; built locally with `build_llm_judge_labels.py` |
| **Inter-rater reliability (IRR)** | Agreement between registry labels and LLM judge; Cohen's κ from `scripts/eval_irr.py` on non-abstain items |
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

## Cost estimate terms

Canonical definitions for cost and savings terminology. Prefer these in README, reference docs, CLI output, and benchmarks.

| Term | JSON / output field | Definition |
| --- | --- | --- |
| **Current cost** | `current_cost`, `project_p50_usd` | Estimated total monthly spend across all dbt models; each model counted once (deduplicated, Fenton–Wilkinson aggregation). |
| **Post-fix cost** | `post_fix_cost` | Counterfactual estimated monthly spend if every current finding were fixed. |
| **Potential savings** | `potential_savings` | Project-level counterfactual reduction: `current_cost − post_fix_cost`, computed once per model (top-down). |
| **Addressable finding savings (deduplicated)** | `savings_p50_usd` | Bottom-up sum of each finding's attributed savings, capped per model so overlapping findings never exceed ~95% of a model's cost. Gates `--fail-on-cost-delta` / `fail_on_monthly_delta`. |
| **Per-finding Est. savings** | `savings_p*_usd_per_month` | Attributed share of savings for one finding. |
| **Model monthly cost** | `model_monthly_p50_usd` | Baseline monthly cost of the model the finding sits on—not the saving. |
| **Post-fix cost per finding** | `post_fix_cost_p50_usd_per_month` | That model's modeled monthly cost after fixing the finding. |
| **Relative index** | `relative_index` | Savings in GB-months when pricing is disabled. |
| **Grade A / B / C** | `grade`, `grade_a` / `grade_b` / `grade_c` | Input provenance: **A** measured (observations or query history), **B** catalog/config, **C** size prior only. |
| **Coverage / mapped spend** | `coverage.mapped_spend_fraction` | Fraction of models backed by measured (grade A) spend. |
| **PR impact** | `pr_impact` | Base-vs-head deltas in PR mode: `introduced`, `avoided`, `net`, `efficiency`, `volume`. |
| **Realized savings** | `realized_savings` | Measured before/after delta from `observations_before` + `observations_after` bundles. |

**Two savings numbers:** **Potential savings** is top-down (`current − post_fix`, per model). **Addressable finding savings** is bottom-up (sum of per-finding shares with structure/fan-out weights and per-model caps). They are close but not identical; PR/scan cost gating uses the finding sum.

**Disclaimer:** Cost figures are advisory priors from local files only. They prioritize findings at order-of-magnitude fidelity—they are not a bill and not a guarantee of realized savings. Severity and confidence are the default gate; calibrated cost gates are explicit opt-ins. Full disclaimer: [Cost estimates](reference/cost-estimates.md).

## Documentation style

- Use imperative headings in reference pages
- Prefix script invocations with `python3`
- Show `--warehouse` on scan examples even when optional
- CI examples: prefer `--base origin/main` with footnote that CLI default is `main`

## Related

- [Benchmark tiers](contributing/benchmark-tiers.md)
- [Parse metrics](reference/parse-metrics.md)
- [Platforms](reference/platforms.md)

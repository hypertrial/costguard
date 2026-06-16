# Spellbook top-10 cost findings — manual review

Generated: 2026-06-16  
Scan: spellbook @ `e3a7dc9e8a49fa8a9a55d967fa8cabe5dd963738`, `cost=true`, `warehouse=trino`  
Source: 10,809 diagnostics; 3,756 with `cost_estimate`; ranked by `savings_p50_usd_per_month` then `relative_index` (same as Costguard cost report).

Raw extract: `/tmp/spellbook_top10_cost.json`

## Findings

| # | Rule | Path:line | Savings p50 | Relative index | Registry bucket | Manual verdict | Rationale |
|---|------|-----------|-------------|----------------|-----------------|----------------|-----------|
| 1 | SQLCOST015 | `tokens_abstract_erc20_stablecoins_core.sql:13` | — | 2600 | other (tp) | **FP** | Fires on comment text `core list: frozen` matched as a pseudo-JSON `word: word` pattern by `json_regex`; SQL is a static `VALUES` seed table, not a repeated expensive expression. |
| 2 | SQLCOST015 | `tokens_abstract_erc20_stablecoins_extended.sql:13` | — | 2600 | other (tp) | **FP** | Same false match on `extended list: new` in the header comment; trivial one-row `VALUES` lookup table. |
| 3 | SQLCOST015 | `tokens_arbitrum_erc20_stablecoins_core.sql:13` | — | 2600 | other (tp) | **FP** | Same `core list: frozen` comment false positive; line 13 is a hex literal row, not JSON/regex/normalization. |
| 4 | SQLCOST015 | `tokens_arbitrum_erc20_stablecoins_extended.sql:13` | — | 2600 | other (tp) | **FP** | Same `extended list: new` comment pattern across chain templates; no cross-file expensive compute to materialize. |
| 5 | SQLCOST015 | `tokens_avalanche_c_erc20_stablecoins_core.sql:13` | — | 2600 | other (tp) | **FP** | Identical boilerplate comment triggers project-wide duplicate “expression” key; static address list only. |
| 6 | SQLCOST015 | `tokens_avalanche_c_erc20_stablecoins_extended.sql:13` | — | 2600 | other (tp) | **FP** | Same extended-list comment false positive as #2/#4. |
| 7 | SQLCOST015 | `tokens_base_erc20_stablecoins_core.sql:13` | — | 2600 | other (tp) | **FP** | Same `core list: frozen` comment false positive as #1/#3. |
| 8 | SQLCOST015 | `tokens_base_erc20_stablecoins_extended.sql:13` | — | 2600 | other (tp) | **FP** | Same `extended list: new` comment false positive. |
| 9 | SQLCOST015 | `tokens_berachain_erc20_stablecoins_core.sql:13` | — | 2600 | other (tp) | **FP** | Same `core list: frozen` comment false positive. |
| 10 | SQLCOST015 | `tokens_berachain_erc20_stablecoins_extended.sql:13` | — | 2600 | other (tp) | **FP** | Same `extended list: new` comment false positive. |

## Summary

| Metric | Count |
|--------|-------|
| Costguard correct (TP) | **0 / 10** |
| False positives | **10 / 10** |
| Uncertain | **0 / 10** |

**Rules represented:** SQLCOST015 only (100%).

**Registry agreement:** 0 / 10 — `fp_registry.toml` marks bucket `other` as TP for cross-file expensive expressions; manual review disagrees on all ten.

## Root cause

SQLCOST015 ([`RepeatedExpensiveAcrossFilesRule`](../../crates/costguard-rules/src/expressions.rs)) counts duplicate feature keys from `json_extractions`, `regex_calls`, and `normalization_calls` across files (threshold ≥ 3).

The stablecoin seed models share templated header comments:

- `-- core list: frozen stablecoin addresses …`
- `-- extended list: new stablecoin addresses …`

The JSON feature regex ([`json_regex`](../../crates/costguard-sql/src/features/regex.rs)) includes `\b\w+\s*:\s*…`, which matches **`list: frozen`** and **`list: new`** inside comments. That produces a spurious cross-file “expensive expression” key even though the SQL body is only `SELECT … FROM (VALUES …)`.

Diagnostic line numbers point at the first `VALUES` row (~line 13) because that is where the matched span lands in the file, not because the hex literal is expensive.

## Cost ranking caveat

All ten share `relative_index = 2600` and `savings_p50_usd_per_month = null`. They rank at the top only because many findings lack dollar estimates; the cost basis string assumes 50 GB × 30 runs/mo on static tables — overstated for these models.

## Takeaways

1. **Top “cost savings” findings can be entirely false positives** when the cost ranker and SQLCOST015 share the same feature-extraction bug.
2. **Comment-aware JSON detection** is needed: strip or ignore `--` comments before applying the `\w+:` JSON-path heuristic, or tighten the alternation so it does not match prose colons in comments.
3. **Stablecoin seed templates** (~40+ chains × core/extended) likely generate hundreds of similar SQLCOST015 FPs beyond this top-10 slice.
4. **Candidate for rule tuning (v4, not filed here):** exempt static `VALUES` seed models tagged `static`, or require matched features to appear outside comments/literals.

## Resolution (2026-06-16)

Fixed in [`crates/costguard-sql/src/features/regex.rs`](../../crates/costguard-sql/src/features/regex.rs): expensive-expression regexes (`json_extractions`, `regex_calls`, `normalization_calls`) now run on comment-masked SQL (length-preserving `--`, `/* */`, and `{# #}` masking). Spellbook smoke baseline SQLCOST015 dropped from 229 → 13; full spellbook 785 → 330. The stablecoin seed-model FPs documented above are resolved.

## Adjudication method

For each finding: read compiled SQL via manifest, check rule rubric ([`docs/rules/SQLCOST015.md`](../rules/SQLCOST015.md)), classify bucket via [`scripts/bucket_rule_diagnostics.py`](../../scripts/bucket_rule_diagnostics.py), compare to [`tests/benchmarks/fp_registry.toml`](../../tests/benchmarks/fp_registry.toml), and trace the triggering feature key against [`json_regex`](../../crates/costguard-sql/src/features/regex.rs).

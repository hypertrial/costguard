# Binary classification metrics

Costguard rule evaluation uses a frozen labeled dataset and standard binary-classification metrics. This complements the legacy precision-only Spellbook triage workflow.

## Unit of classification

Each row in [`tests/benchmarks/eval_labels.toml`](../../tests/benchmarks/eval_labels.toml) is one decision:

`(repo@sha, path, rule) → y_true`

- **Corpus split:** `path` is a corpus case directory (for example `incremental_missing`). `y_true = 1` when the case lists the rule in `expect_rules`; `y_true = 0` when listed in `forbid_rules`.
- **Real split:** most rows use `path = __bucket__:{bucket}` templates seeded from [`fp_registry.toml`](../../tests/benchmarks/fp_registry.toml). Findings are bucketed with the same regex classifiers as `bucket_rule_diagnostics.py`.

`y_pred = 1` when Costguard emits the rule for that unit; otherwise `0`.

## Score axis (ranking metrics)

PR-AUC and ROC-AUC use a per-finding score:

1. `cost_estimate.current_cost_p50_usd_per_month` (or savings/model monthly p50) when `--cost` is enabled
2. Ordinal `severity × confidence` fallback when cost is unavailable

Non-fired units score `0`.

## Metrics

| Metric | Role |
| --- | --- |
| **Confusion matrix** | TP / FP / TN / FN per split |
| **Precision / recall / F1** | Standard detection quality |
| **MCC** | Headline metric under class imbalance |
| **Balanced accuracy** | Mean of per-class recall |
| **PR-AUC** | Headline ranking metric (preferred over ROC-AUC here) |
| **ROC-AUC** | Secondary ranking metric |
| **Wilson CI** | 95% intervals on precision and recall |

Plain accuracy is intentionally omitted (misleading when negatives dominate).

## Splits and gating

| Split | Labels | Gate |
| --- | --- | --- |
| `corpus` | Authored gold from corpus manifest | Hard: precision/recall/MCC = 1.0 |
| `real` | Provisional bucket templates from fp_registry | Soft: precision floors until human review |

The corpus split is deterministic and runs in every full local CI gate; the per-change `--fast` mode defers it to manual release qualification. The real split requires a cached Spellbook checkout and runs behind `./scripts/ci_local.sh --precision`.

## Commands

Install eval dependencies once:

```bash
python3 -m venv .venv-eval
.venv-eval/bin/pip install --require-hashes -r requirements-eval.lock
```

Regenerate the frozen dataset:

```bash
python3 scripts/build_eval_dataset.py --write
python3 scripts/build_eval_dataset.py --write --sample-negatives 200  # optional TN stubs
```

Evaluate:

```bash
.venv-eval/bin/python scripts/eval_metrics.py --split corpus
.venv-eval/bin/python scripts/eval_metrics.py --split real
.venv-eval/bin/python scripts/eval_metrics.py --split all --json-out triage/eval.json
```

## Label sources

| Source | Meaning |
| --- | --- |
| `seed:corpus` | Gold labels from corpus `expect_rules` / `forbid_rules` |
| `seed:fp_registry` | Provisional bucket verdicts; needs human review for headline real metrics |
| `seed:negative_sample` | Sampled non-fired `(path, rule)` pairs; `pending = true` until labeled |
| `human` | Per-finding adjudication (follow-up) |
| `cost` | Semi-objective labels from warehouse cost data (follow-up) |

## Follow-up: human labeling protocol

1. Stratified sample fired findings and non-fired `(model, rule)` pairs from pinned external repos.
2. Record per-finding labels keyed by `(repo@sha, path, line, rule, finding_id)`.
3. Double-label a 10% subset; report Cohen's κ before promoting real-split gates from soft to hard. See [LLM judge IRR](llm-judge-irr.md) for the automated local second-rater pipeline (`build_llm_judge_labels.py` + `eval_irr.py`).
4. Replace bucket-template rows in `eval_labels.toml` with per-finding rows as labels mature.

## Related

- [`scripts/eval_metrics.py`](../../scripts/eval_metrics.py) — metric computation and gates
- [`scripts/build_eval_dataset.py`](../../scripts/build_eval_dataset.py) — dataset seeding
- [`scripts/recall_report.py`](../../scripts/recall_report.py) — corpus **coverage** gate (not operational recall)
- [`scripts/precision_triage.py`](../../scripts/precision_triage.py) — legacy sampled precision workflow
- [LLM judge IRR](llm-judge-irr.md) — local second-rater labels and Cohen's κ (`eval_irr.py`)

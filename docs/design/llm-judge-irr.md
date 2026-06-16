# LLM-as-judge inter-rater reliability

Costguard's frozen `eval_labels.toml` real split is seeded from `fp_registry.toml` bucket verdicts — effectively a single human/registry rater. This pipeline adds a **local LLM second rater** and measures agreement with Cohen's κ.

The judge **never runs in CI**. Developers run the build tool locally, commit the cached JSONL labels, and CI only validates consistency and recomputes κ.

## Components

| Artifact | Purpose |
| --- | --- |
| `scripts/build_llm_judge_labels.py` | Local dataset-build tool (requires GGUF + llama-cpp-python) |
| `scripts/eval_irr.py` | CI-safe validator + κ reporter |
| `tests/benchmarks/llm_judge_labels.jsonl` | Committed per-finding judge verdicts |
| `tests/benchmarks/llm_judge_manifest.toml` | Pinned model/runtime/prompt configuration |
| `tests/benchmarks/irr_report.json` | Latest κ report (regenerated in CI) |

Shared helpers live in `scripts/llm_judge_lib.py`.

## Recommended local setup (32 GB M4 Air)

| Component | Choice |
| --- | --- |
| Runtime | **llama-cpp-python**, Metal (`n_gpu_layers=-1`) |
| Model | **Qwen3-30B-A3B-Instruct-2507**, Q4_K_M GGUF |
| Context | **32768** tokens (16384 for dense Qwen3-32B fallback) |
| Output | Single constrained token: `A` / `B` / `C` |
| Sampling | `temperature=0`, fixed seed `3407` |

Fallback order: Qwen3-30B-A3B Q4 → Qwen3-32B Q4 → Gemma 3 27B-it Q4.

## Judging strategy

The model is **not** asked for free-form rationale. Each finding receives a one-token classification:

| Token | Verdict |
| --- | --- |
| `A` | true positive |
| `B` | false positive |
| `C` | unsure |

**Logprob-margin abstention:** if `abs(logp_A - logp_B) < 0.5`, the verdict is forced to `unsure` even when the generated token is `A` or `B`. Do not map `unsure` to `fp` when computing κ.

Prompt version: `irr_judge_v1`. The prompt includes rule id/title/description/rubric (from `costguard rules --format json` + `docs/rules/<id>.md`), dialect, finding line/span/message, and SQL. It **excludes** registry bucket names, existing `y_true`, and triage rationale.

## SQL context packing

Deterministic, no LLM summarization:

1. Full SQL if within budget.
2. Else: finding line ± 40, containing CTE/model block, referenced CTE definitions.
3. If still too large: record `unsure_due_to_context_limit`.

Budget uses a chars/token heuristic (`context_tokens * 3`).

## Sampling scope

- Repo: **spellbook** (default)
- Granularity: per fired finding
- Cap: **50 findings per (rule, bucket)**, deterministic seed **3407**
- `registry_verdict` from committed real-split labels (`eval_labels.toml`)

## Cache key

Each JSONL record stores a `cache_key` = SHA256 of:

```
finding_id + rule_id + rule_description_sha + sql_sha + finding_span
+ prompt_version + model_file_sha256 + runtime_version
```

Stale entries are not silently reused when prompt, model, or SQL changes.

## κ reporting

`eval_irr.py` reports on the **non-abstain** subset where both raters gave `tp` or `fp`:

- `kappa_binary_non_abstain` (Cohen's κ)
- coverage, abstain rate, disagreement rate
- per-rule κ (rules with ≥ 5 scorable samples)
- top disagreement `(rule, bucket)` pairs

CI is **report-only** — no κ floor gate yet.

## Reproduction

```bash
# One-time local judge env (not used by CI)
python3 -m venv .venv-judge
.venv-judge/bin/pip install -r requirements-judge.txt

# Ensure spellbook benchmark cache exists
python3 scripts/benchmark_external_repo.py --repo spellbook

export COSTGUARD_JUDGE_GGUF=/path/to/Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf
.venv-judge/bin/python scripts/build_llm_judge_labels.py --model "$COSTGUARD_JUDGE_GGUF"

# CI-safe validation + κ (uses .venv-eval)
.venv-eval/bin/python scripts/eval_irr.py
```

Dry-run candidate enumeration without the model:

```bash
python3 scripts/build_llm_judge_labels.py --dry-run
```

## Related docs

- [Classification metrics](classification-metrics.md) — operational precision/recall/MCC for Costguard predictions
- [Benchmark calibration](benchmark-calibration.md) — corpus and external benchmark layers

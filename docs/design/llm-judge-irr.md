# LLM-as-judge inter-rater reliability

> **Note:** This doc covers the **internal IRR benchmark judge** (calibration against `fp_registry.toml`). A user-facing judge is advisory and deferred; see [Product scope](product-scope.md#offline-llm-judge-advisory-and-deferred).

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
| Runtime | **llama-cpp-python**, Metal (`n_gpu_layers=-1`), `flash_attn=true` |
| Model | **Qwen3-30B-A3B-Instruct-2507**, Q4_K_M GGUF |
| Context | **32768** tokens default |
| Batch | `n_batch=2048`, `n_ubatch=512` |
| SQL target | **~8000 tokens** per file (`sql_token_target`) |
| Prompting | **ChatML** via `create_chat_completion` (GGUF embedded template) |
| Output | Structured JSON verdict (~32 tokens): `exemption_applies`, `failure_condition_met`, `verdict` |
| Sampling | `temperature=0`, fixed seed `3407`, concurrency **1** |

Fallback order: Qwen3-30B-A3B Q4 → Qwen3-32B Q4 → Gemma 3 27B-it Q4.

## Performance: prefill amortization

The bottleneck is **prompt prefill**, not decode. On an M4 Air 32 GB with Qwen3-30B-A3B Q4, expect roughly:

| Metric | Estimate |
| --- | --- |
| Prompt prefill | 150–350 tokens/sec sustained |
| Short decode | 20–45 tokens/sec |
| Verdict output | ~0.05–0.3 sec |

For ~1,200 spellbook findings with sane packed prompts (~2–8k tokens/file):

| Strategy | Expected runtime |
| --- | --- |
| Default: per-file SQL + KV prefix reuse | **4–12 hours** |
| `--grouped`: one call per file | **1–6 hours** |
| Naive per-finding full compiled SQL | **12–55+ hours** |

### Default mode (`mode = prefix`, prompt `irr_judge_v3`)

1. Group capped findings by SQL file (`path`).
2. Build **one shared SQL context per file** via deterministic union excerpt packing.
3. Sort findings by `(path, rule, line)` so consecutive calls share the `system + SQL` chat prefix.
4. Reuse KV cache (`LlamaRAMCache`) across per-finding structured JSON calls via `create_chat_completion`.
5. **Checkpoint** after each file (default `--checkpoint-every 1`) so long runs resume without losing progress.

### Grouped mode (`--grouped`, prompt `irr_judge_v3+grouped`)

One LLM call per file returns a JSON verdict array for all findings in that file. Faster prefill amortization; abstention when the model emits `C` (`abstention_reason=model_unsure`).

## Judging strategy

The model is **not** asked for free-form rationale. Default mode returns a small JSON object (GBNF-constrained):

| Field | Meaning |
| --- | --- |
| `exemption_applies` | Documented rubric exemption applies |
| `failure_condition_met` | Rule failure condition clearly met in SQL |
| `verdict` | `A` / `B` / `C` letter |

Deterministic post-processing (`map_structured_verdict`) maps fields to `tp` / `fp` / `unsure`:

- `C` → `unsure` (`model_unsure`)
- `exemption_applies=true` → `fp`
- `failure_condition_met=true` (and not exempt) → `tp`
- else → `fp`

Per-rule **few-shot exemplars** live in `tests/benchmarks/judge_fewshots.toml` (SQLCOST012, SQLCOST017, SQLCOST006, SQLCOST014). Their SHA is part of `cache_key`.

**Abstention:** when the model emits `C`, verdict is `unsure` with `abstention_reason=model_unsure`. Truncated SQL contexts also force `unsure` (`context_truncated`). Do not map `unsure` to `fp` when computing κ.

The v3 prompt is **balanced** — no global default-to-B — so the model must apply rubric exemptions and failure conditions literally.

Prompt versions:

| Mode | `prompt_version` |
| --- | --- |
| Default (prefix reuse) | `irr_judge_v3` |
| Grouped JSON | `irr_judge_v3+grouped` |

The user message puts **SQL before per-finding rule context** so the shared prefix is stable across findings in a file. It **excludes** registry bucket names, existing `y_true`, and triage rationale.

## SQL context packing

Deterministic, no LLM summarization. Per **file** (not per finding):

1. Full SQL if within `sql_token_target` (~8000 tokens).
2. Else: deduped union of finding line ± 40, containing CTE/model blocks, referenced CTE definitions.
3. If still too large: all findings in that file record `unsure_due_to_context_limit`.

Budget uses a chars/token heuristic (`sql_token_target * 3`).

## Sampling scope

- Repo: **spellbook** (default)
- Granularity: per fired finding
- Cap: **50 findings per (rule, bucket)**, deterministic seed **3407**
- Optional `--rule-id` filter for fast iteration on specific rules
- `registry_verdict` from committed real-split labels (`eval_labels.toml`)

## Cache key

Each JSONL record stores a `cache_key` = SHA256 of:

```
finding_id + rule_id + rule_description_sha + sql_sha + finding_span
+ prompt_version + model_file_sha256 + runtime_version + mode + fewshots_sha
```

Stale entries are not silently reused when prompt, model, SQL, or mode changes.

## κ reporting

`eval_irr.py` reports on the **non-abstain** subset where both raters gave `tp` or `fp`:

- `kappa_binary_non_abstain` (Cohen's κ) — co-primary with FP recall under class skew
- `mcc` (Matthews correlation)
- `registry_fp_recall` — P(judge=fp \| registry=fp)
- `registry_tp_recall` — P(judge=tp \| registry=tp)
- `registry_fp_precision` — P(registry=fp \| judge=fp)
- coverage, abstain rate, disagreement rate
- per-rule κ, MCC, and FP recall (rules with ≥ 5 scorable samples)
- top disagreement `(rule, bucket)` pairs

CI is **report-only** — no κ floor gate yet.

## Reproduction

```bash
# One-time local judge env (not used by CI)
python3 -m venv .venv-judge
.venv-judge/bin/pip install --require-hashes -r requirements-judge.lock

# Ensure spellbook benchmark cache exists
python3 scripts/benchmark_external_repo.py --repo spellbook

export COSTGUARD_JUDGE_GGUF=/path/to/Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf

# Default: per-file SQL + KV prefix reuse (recommended overnight run)
.venv-judge/bin/python scripts/build_llm_judge_labels.py --model "$COSTGUARD_JUDGE_GGUF"

# Fast iteration on one rule
.venv-judge/bin/python scripts/build_llm_judge_labels.py \
  --model "$COSTGUARD_JUDGE_GGUF" --rule-id SQLCOST012 --cap 10 \
  --out /tmp/judge_subset.jsonl --manifest-out /tmp/judge_subset_manifest.toml

# Faster: one JSON call per file
.venv-judge/bin/python scripts/build_llm_judge_labels.py --model "$COSTGUARD_JUDGE_GGUF" --grouped

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

#!/usr/bin/env python3
"""Build committed LLM judge labels for inter-rater reliability (local only)."""

from __future__ import annotations

import argparse
import os
import random
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from bucket_rule_diagnostics import (  # noqa: E402
    CLASSIFIERS,
    load_manifest_sql,
    read_sql_for_diagnostic,
)
from costguard_tooling import (  # noqa: E402
    file_sha256,
    repo_by_name,
    run_costguard_scan,
)
from eval_lib import (  # noqa: E402
    DEFAULT_LABELS,
    load_eval_labels,
    normalize_path,
    repo_checkout,
)
from llm_judge_lib import (  # noqa: E402
    DEFAULT_CAP,
    DEFAULT_CONTEXT_TOKENS,
    DEFAULT_FLASH_ATTN,
    DEFAULT_LABELS_JSONL,
    DEFAULT_MANIFEST,
    DEFAULT_MAX_NEW_TOKENS,
    DEFAULT_MODEL_ID,
    DEFAULT_N_BATCH,
    DEFAULT_N_UBATCH,
    DEFAULT_QUANT,
    DEFAULT_RUNTIME,
    DEFAULT_SEED,
    DEFAULT_SQL_TOKEN_TARGET,
    JUDGE_NAME,
    JUDGE_VERSION,
    MODE_GROUPED,
    MODE_PREFIX,
    FindingPromptInput,
    JudgeManifest,
    JudgeRecord,
    LlamaJudge,
    RuleMetadata,
    StructuredVerdict,
    build_grouped_messages,
    build_messages,
    cache_key,
    candidate_sort_key,
    fewshots_file_sha,
    finding_id_for_diagnostic,
    finding_span,
    load_judge_records,
    load_rule_metadata,
    messages_sha256,
    pack_sql_for_file,
    prompt_version_for_mode,
    runtime_version,
    utc_now_iso,
    verdict_from_letter,
    verdict_from_structured,
    verify_gguf_chat_template,
    write_judge_records,
    write_manifest,
)


@dataclass(frozen=True)
class CandidateFinding:
    repo: str
    diagnostic: dict[str, Any]
    bucket: str
    registry_verdict: str | None


@dataclass(frozen=True)
class FileContext:
    path: str
    sql: str
    sql_sha: str
    truncated: bool
    too_large: bool
    candidates: list[CandidateFinding]


def bucket_verdict_map(repo_name: str, labels_path: Path) -> dict[tuple[str, str], str]:
    mapping: dict[tuple[str, str], str] = {}
    for label in load_eval_labels(labels_path):
        if label.repo != repo_name or label.split != "real":
            continue
        bucket = label.bucket()
        if bucket is None or label.y_true is None:
            continue
        mapping[(label.rule, bucket)] = "tp" if label.y_true == 1 else "fp"
    return mapping


def scan_repo(repo_name: str, cache: Path) -> tuple[dict[str, Any], Path, dict[str, str], str]:
    repo = repo_by_name(repo_name)
    checkout = repo_checkout(repo_name, cache)
    manifest = checkout / "target" / "manifest.json"
    if not manifest.exists():
        raise SystemExit(
            f"missing manifest at {manifest}; run benchmark_external_repo.py --repo {repo_name} first"
        )
    payload, _ = run_costguard_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        fail_on="critical",
        manifest=manifest,
        cost=bool(repo.get("cost", False)),
    )
    compiled = load_manifest_sql(manifest)
    return payload, checkout, compiled, str(repo.get("warehouse", "generic"))


def collect_candidates(
    repo_name: str,
    cache: Path,
    labels_path: Path,
) -> tuple[list[CandidateFinding], Path, dict[str, str], str]:
    payload, checkout, compiled, dialect = scan_repo(repo_name, cache)
    verdicts = bucket_verdict_map(repo_name, labels_path)
    candidates: list[CandidateFinding] = []
    for diagnostic in payload.get("diagnostics", []):
        rule_id = str(diagnostic.get("rule_id", ""))
        if not rule_id:
            continue
        sql = read_sql_for_diagnostic(checkout, diagnostic, compiled)
        classifier = CLASSIFIERS.get(rule_id, lambda _sql: "other")
        bucket = classifier(sql)
        registry_verdict = verdicts.get((rule_id, bucket))
        candidates.append(
            CandidateFinding(
                repo=repo_name,
                diagnostic=diagnostic,
                bucket=bucket,
                registry_verdict=registry_verdict,
            )
        )
    return candidates, checkout, compiled, dialect


def cap_candidates(
    candidates: list[CandidateFinding],
    *,
    cap: int,
    seed: int,
) -> list[CandidateFinding]:
    grouped: dict[tuple[str, str], list[CandidateFinding]] = defaultdict(list)
    for candidate in candidates:
        rule_id = str(candidate.diagnostic.get("rule_id", ""))
        grouped[(rule_id, candidate.bucket)].append(candidate)

    rng = random.Random(seed)
    capped: list[CandidateFinding] = []
    for key in sorted(grouped):
        group = grouped[key]
        if len(group) <= cap:
            capped.extend(group)
        else:
            capped.extend(rng.sample(group, cap))
    return sorted(capped, key=candidate_sort_key)


def build_file_contexts(
    candidates: list[CandidateFinding],
    *,
    checkout: Path,
    compiled: dict[str, str],
    manifest: JudgeManifest,
) -> list[FileContext]:
    by_path: dict[str, list[CandidateFinding]] = defaultdict(list)
    for candidate in candidates:
        rel_path = normalize_path(str(candidate.diagnostic.get("path", "")))
        by_path[rel_path].append(candidate)

    contexts: list[FileContext] = []
    for path in sorted(by_path):
        group = by_path[path]
        first = group[0].diagnostic
        sql_raw = read_sql_for_diagnostic(checkout, first, compiled)
        lines = [int(item.diagnostic.get("line", 0) or 0) for item in group]
        packed, sql_sha, truncated, too_large = pack_sql_for_file(
            sql_raw,
            lines,
            sql_token_target=manifest.sql_token_target,
        )
        contexts.append(
            FileContext(
                path=path,
                sql=packed,
                sql_sha=sql_sha,
                truncated=truncated,
                too_large=too_large,
                candidates=group,
            )
        )
    return contexts


def make_record(
    candidate: CandidateFinding,
    *,
    file_ctx: FileContext,
    dialect: str,
    rule_meta: RuleMetadata,
    manifest: JudgeManifest,
    model_sha256: str,
    runtime_ver: str,
    llm_verdict: str,
    label_token: str,
    logprobs: dict[str, float],
    abstention_reason: str | None,
    input_sha256: str,
    structured: StructuredVerdict | None = None,
    fewshots_sha: str = "",
) -> JudgeRecord:
    diagnostic = candidate.diagnostic
    rule_id = str(diagnostic.get("rule_id", ""))
    rel_path = normalize_path(str(diagnostic.get("path", "")))
    line = int(diagnostic.get("line", 0) or 0)
    message = str(diagnostic.get("message", ""))
    finding_id = finding_id_for_diagnostic(candidate.repo, diagnostic)
    span = finding_span(diagnostic)
    rule_description_sha = rule_meta.description_sha
    key = cache_key(
        finding_id=finding_id,
        rule_id=rule_id,
        rule_description_sha=rule_description_sha,
        sql_sha=file_ctx.sql_sha,
        finding_span=span,
        prompt_version=manifest.prompt_version,
        model_file_sha256=model_sha256,
        runtime_version=runtime_ver,
        mode=manifest.mode,
        fewshots_sha=fewshots_sha,
    )
    return JudgeRecord(
        finding_id=finding_id,
        rule_id=rule_id,
        repo=candidate.repo,
        path=rel_path,
        line=line,
        bucket=candidate.bucket,
        registry_verdict=candidate.registry_verdict,
        llm_verdict=llm_verdict,
        label_token=label_token,
        model=manifest.model_id,
        quant=manifest.quantization,
        runtime=manifest.runtime,
        prompt_version=manifest.prompt_version,
        input_sha256=input_sha256,
        model_sha256=model_sha256,
        cache_key=key,
        created_at=utc_now_iso(),
        logprobs=logprobs,
        abstention_reason=abstention_reason,
        context_truncated=file_ctx.truncated,
        rule_description_sha=rule_description_sha,
        sql_sha=file_ctx.sql_sha,
        finding_span=span,
        runtime_version=runtime_ver,
        message=message,
        dialect=dialect,
        mode=manifest.mode,
        exemption_applies=structured.exemption_applies if structured else None,
        failure_condition_met=structured.failure_condition_met if structured else None,
        raw_verdict_json=structured.raw_json if structured else "",
        fewshots_sha=fewshots_sha,
    )


def judge_file_prefix(
    file_ctx: FileContext,
    *,
    dialect: str,
    rule_meta_map: dict[str, RuleMetadata],
    manifest: JudgeManifest,
    model_sha256: str,
    judge: LlamaJudge,
    cache: dict[str, JudgeRecord],
    fewshots_sha: str,
) -> list[JudgeRecord]:
    runtime_ver = runtime_version()
    records: list[JudgeRecord] = []
    for candidate in file_ctx.candidates:
        diagnostic = candidate.diagnostic
        rule_id = str(diagnostic.get("rule_id", ""))
        rule_meta = rule_meta_map.get(rule_id)
        if rule_meta is None:
            raise SystemExit(f"missing rule metadata for {rule_id}")
        line = int(diagnostic.get("line", 0) or 0)
        message = str(diagnostic.get("message", ""))
        span = finding_span(diagnostic)
        finding_id = finding_id_for_diagnostic(candidate.repo, diagnostic)
        rule_description_sha = rule_meta.description_sha
        key = cache_key(
            finding_id=finding_id,
            rule_id=rule_id,
            rule_description_sha=rule_description_sha,
            sql_sha=file_ctx.sql_sha,
            finding_span=span,
            prompt_version=manifest.prompt_version,
            model_file_sha256=model_sha256,
            runtime_version=runtime_ver,
            mode=manifest.mode,
            fewshots_sha=fewshots_sha,
        )
        cached = cache.get(key)
        if cached is not None:
            records.append(cached)
            continue

        if file_ctx.too_large:
            system, user = build_messages(
                rule_meta,
                dialect=dialect,
                line=line,
                span=span,
                message=message,
                sql=file_ctx.sql,
            )
            records.append(
                make_record(
                    candidate,
                    file_ctx=file_ctx,
                    dialect=dialect,
                    rule_meta=rule_meta,
                    manifest=manifest,
                    model_sha256=model_sha256,
                    runtime_ver=runtime_ver,
                    llm_verdict="unsure",
                    label_token="C",
                    logprobs={},
                    abstention_reason="unsure_due_to_context_limit",
                    input_sha256=messages_sha256(system, user),
                    fewshots_sha=fewshots_sha,
                )
            )
            continue

        system, user = build_messages(
            rule_meta,
            dialect=dialect,
            line=line,
            span=span,
            message=message,
            sql=file_ctx.sql,
        )
        structured = judge.judge(system, user, max_tokens=manifest.max_new_tokens)
        llm_verdict, abstention_reason, label_token = verdict_from_structured(structured)
        if file_ctx.truncated and llm_verdict in {"tp", "fp"} and abstention_reason is None:
            llm_verdict = "unsure"
            abstention_reason = "context_truncated"
        records.append(
            make_record(
                candidate,
                file_ctx=file_ctx,
                dialect=dialect,
                rule_meta=rule_meta,
                manifest=manifest,
                model_sha256=model_sha256,
                runtime_ver=runtime_ver,
                llm_verdict=llm_verdict,
                label_token=label_token,
                logprobs={},
                abstention_reason=abstention_reason,
                input_sha256=messages_sha256(system, user),
                structured=structured,
                fewshots_sha=fewshots_sha,
            )
        )
    return records


def judge_file_grouped(
    file_ctx: FileContext,
    *,
    dialect: str,
    rule_meta_map: dict[str, RuleMetadata],
    manifest: JudgeManifest,
    model_sha256: str,
    judge: LlamaJudge,
    cache: dict[str, JudgeRecord],
    fewshots_sha: str,
) -> list[JudgeRecord]:
    runtime_ver = runtime_version()
    if file_ctx.too_large:
        return judge_file_prefix(
            file_ctx,
            dialect=dialect,
            rule_meta_map=rule_meta_map,
            manifest=manifest,
            model_sha256=model_sha256,
            judge=judge,
            cache=cache,
            fewshots_sha=fewshots_sha,
        )

    prompt_inputs: list[FindingPromptInput] = []
    for index, candidate in enumerate(file_ctx.candidates):
        rule_id = str(candidate.diagnostic.get("rule_id", ""))
        rule_meta = rule_meta_map.get(rule_id)
        if rule_meta is None:
            raise SystemExit(f"missing rule metadata for {rule_id}")
        prompt_inputs.append(
            FindingPromptInput(
                index=index,
                rule_meta=rule_meta,
                line=int(candidate.diagnostic.get("line", 0) or 0),
                span=finding_span(candidate.diagnostic),
                message=str(candidate.diagnostic.get("message", "")),
            )
        )

    system, user = build_grouped_messages(prompt_inputs, dialect=dialect, sql=file_ctx.sql)
    input_sha256 = messages_sha256(system, user)

    uncached: list[tuple[int, CandidateFinding, RuleMetadata]] = []
    records_by_index: dict[int, JudgeRecord] = {}
    for index, candidate in enumerate(file_ctx.candidates):
        rule_id = str(candidate.diagnostic.get("rule_id", ""))
        rule_meta = rule_meta_map[rule_id]
        finding_id = finding_id_for_diagnostic(candidate.repo, candidate.diagnostic)
        span = finding_span(candidate.diagnostic)
        key = cache_key(
            finding_id=finding_id,
            rule_id=rule_id,
            rule_description_sha=rule_meta.description_sha,
            sql_sha=file_ctx.sql_sha,
            finding_span=span,
            prompt_version=manifest.prompt_version,
            model_file_sha256=model_sha256,
            runtime_version=runtime_ver,
            mode=manifest.mode,
            fewshots_sha=fewshots_sha,
        )
        cached = cache.get(key)
        if cached is not None:
            records_by_index[index] = cached
        else:
            uncached.append((index, candidate, rule_meta))

    if uncached:
        letters = judge.judge_grouped(system, user, len(file_ctx.candidates))
        for index, candidate, rule_meta in uncached:
            letter = letters[index]
            llm_verdict, abstention_reason = verdict_from_letter(letter)
            records_by_index[index] = make_record(
                candidate,
                file_ctx=file_ctx,
                dialect=dialect,
                rule_meta=rule_meta,
                manifest=manifest,
                model_sha256=model_sha256,
                runtime_ver=runtime_ver,
                llm_verdict=llm_verdict,
                label_token=letter,
                logprobs={},
                abstention_reason=abstention_reason,
                input_sha256=input_sha256,
                fewshots_sha=fewshots_sha,
            )

    return [records_by_index[index] for index in range(len(file_ctx.candidates))]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="spellbook")
    parser.add_argument("--cache", type=Path, default=Path.home() / ".cache/costguard/benchmarks")
    parser.add_argument("--labels", type=Path, default=DEFAULT_LABELS)
    parser.add_argument("--out", type=Path, default=DEFAULT_LABELS_JSONL)
    parser.add_argument("--manifest-out", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--model", type=Path, default=None, help="Path to local GGUF model")
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--quant", default=DEFAULT_QUANT)
    parser.add_argument("--cap", type=int, default=DEFAULT_CAP)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument("--context-tokens", type=int, default=DEFAULT_CONTEXT_TOKENS)
    parser.add_argument("--n-batch", type=int, default=DEFAULT_N_BATCH)
    parser.add_argument("--n-ubatch", type=int, default=DEFAULT_N_UBATCH)
    parser.add_argument("--sql-token-target", type=int, default=DEFAULT_SQL_TOKEN_TARGET)
    parser.add_argument(
        "--rule-id",
        action="append",
        default=[],
        help="Limit to rule ID(s); repeatable (e.g. --rule-id SQLCOST012)",
    )
    parser.add_argument(
        "--grouped",
        action="store_true",
        help="One LLM call per file with JSON verdict array (no logprob margin)",
    )
    parser.add_argument(
        "--checkpoint-every",
        type=int,
        default=1,
        help="Write labels JSONL after every N files (default 1)",
    )
    parser.add_argument("--dry-run", action="store_true", help="Enumerate candidates only")
    args = parser.parse_args()

    mode = MODE_GROUPED if args.grouped else MODE_PREFIX

    model_path = args.model
    if model_path is None:
        env_model = os.environ.get("COSTGUARD_JUDGE_GGUF")
        if env_model:
            model_path = Path(env_model)
    if model_path is None and not args.dry_run:
        raise SystemExit("missing model path; pass --model or set COSTGUARD_JUDGE_GGUF")

    candidates, checkout, compiled, dialect = collect_candidates(
        args.repo,
        args.cache,
        args.labels,
    )
    capped = cap_candidates(candidates, cap=args.cap, seed=args.seed)
    if args.rule_id:
        allowed = set(args.rule_id)
        capped = [
            item
            for item in capped
            if str(item.diagnostic.get("rule_id", "")) in allowed
        ]
    print(f"Collected {len(candidates)} findings; capped to {len(capped)} ({args.cap}/bucket)")

    if args.dry_run:
        labeled = sum(1 for item in capped if item.registry_verdict is not None)
        file_count = len({normalize_path(str(item.diagnostic.get("path", ""))) for item in capped})
        print(f"Labeled buckets: {labeled}/{len(capped)}")
        print(f"Unique files: {file_count}")
        return 0

    assert model_path is not None
    if not model_path.exists():
        raise SystemExit(f"model file not found: {model_path}")

    verify_gguf_chat_template(model_path)

    model_sha256 = file_sha256(model_path)
    manifest = JudgeManifest(
        judge_name=JUDGE_NAME,
        judge_version=JUDGE_VERSION,
        model_id=args.model_id,
        model_file_sha256=model_sha256,
        quantization=args.quant,
        runtime=DEFAULT_RUNTIME,
        runtime_version=runtime_version(),
        prompt_version=prompt_version_for_mode(mode),
        temperature=0.0,
        seed=args.seed,
        context_tokens=args.context_tokens,
        max_new_tokens=DEFAULT_MAX_NEW_TOKENS,
        cap=args.cap,
        repo=args.repo,
        sample_seed=args.seed,
        n_batch=args.n_batch,
        n_ubatch=args.n_ubatch,
        sql_token_target=args.sql_token_target,
        mode=mode,
        flash_attn=DEFAULT_FLASH_ATTN,
    )

    existing = load_judge_records(args.out)
    cache = {record.cache_key: record for record in existing}
    fewshots_sha = fewshots_file_sha()
    rule_meta_map = load_rule_metadata()
    judge = LlamaJudge(
        model_path,
        n_ctx=manifest.context_tokens,
        seed=manifest.seed,
        n_batch=manifest.n_batch,
        n_ubatch=manifest.n_ubatch,
        flash_attn=manifest.flash_attn,
    )

    file_contexts = build_file_contexts(
        capped,
        checkout=checkout,
        compiled=compiled,
        manifest=manifest,
    )
    records: list[JudgeRecord] = list(existing)
    judged = 0
    files_done = 0
    for file_ctx in file_contexts:
        if manifest.mode == MODE_GROUPED:
            file_records = judge_file_grouped(
                file_ctx,
                dialect=dialect,
                rule_meta_map=rule_meta_map,
                manifest=manifest,
                model_sha256=model_sha256,
                judge=judge,
                cache=cache,
                fewshots_sha=fewshots_sha,
            )
        else:
            file_records = judge_file_prefix(
                file_ctx,
                dialect=dialect,
                rule_meta_map=rule_meta_map,
                manifest=manifest,
                model_sha256=model_sha256,
                judge=judge,
                cache=cache,
                fewshots_sha=fewshots_sha,
            )
        for record in file_records:
            cache[record.cache_key] = record
        records = sorted(cache.values(), key=lambda item: (item.path, item.line, item.finding_id))
        judged += len(file_records)
        files_done += 1
        print(f"judged {judged}/{len(capped)} ({file_ctx.path})")
        if args.checkpoint_every > 0 and files_done % args.checkpoint_every == 0:
            write_judge_records(records, args.out)

    write_judge_records(records, args.out)
    write_manifest(manifest, args.manifest_out)
    print(f"wrote {args.out}")
    print(f"wrote {args.manifest_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

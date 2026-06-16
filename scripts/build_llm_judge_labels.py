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
    DEFAULT_LABELS_JSONL,
    DEFAULT_MANIFEST,
    DEFAULT_MAX_NEW_TOKENS,
    DEFAULT_MODEL_ID,
    DEFAULT_QUANT,
    DEFAULT_RUNTIME,
    DEFAULT_SEED,
    JUDGE_NAME,
    JUDGE_VERSION,
    PROMPT_VERSION,
    JudgeManifest,
    JudgeRecord,
    LlamaJudge,
    build_prompt,
    cache_key,
    decide_verdict,
    finding_id_for_diagnostic,
    finding_span,
    load_judge_records,
    load_rule_metadata,
    pack_sql,
    runtime_version,
    sha256_text,
    utc_now_iso,
    write_judge_records,
    write_manifest,
)


@dataclass(frozen=True)
class CandidateFinding:
    repo: str
    diagnostic: dict[str, Any]
    bucket: str
    registry_verdict: str | None


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
    return capped


def build_record(
    candidate: CandidateFinding,
    *,
    checkout: Path,
    compiled: dict[str, str],
    dialect: str,
    rule_meta_map: dict[str, Any],
    manifest: JudgeManifest,
    model_sha256: str,
    judge: LlamaJudge | None,
    cache: dict[str, JudgeRecord],
) -> JudgeRecord:
    diagnostic = candidate.diagnostic
    rule_id = str(diagnostic.get("rule_id", ""))
    rule_meta = rule_meta_map.get(rule_id)
    if rule_meta is None:
        raise SystemExit(f"missing rule metadata for {rule_id}")

    rel_path = normalize_path(str(diagnostic.get("path", "")))
    line = int(diagnostic.get("line", 0) or 0)
    message = str(diagnostic.get("message", ""))
    finding_id = finding_id_for_diagnostic(candidate.repo, diagnostic)
    span = finding_span(diagnostic)

    sql_raw = read_sql_for_diagnostic(checkout, diagnostic, compiled)
    packed_sql, truncated, too_large = pack_sql(
        sql_raw,
        line,
        context_tokens=manifest.context_tokens,
    )
    sql_sha = sha256_text(packed_sql)
    rule_description_sha = rule_meta.description_sha
    runtime_ver = runtime_version()
    key = cache_key(
        finding_id=finding_id,
        rule_id=rule_id,
        rule_description_sha=rule_description_sha,
        sql_sha=sql_sha,
        finding_span=span,
        prompt_version=manifest.prompt_version,
        model_file_sha256=model_sha256,
        runtime_version=runtime_ver,
    )
    cached = cache.get(key)
    if cached is not None:
        return cached

    if too_large:
        return JudgeRecord(
            finding_id=finding_id,
            rule_id=rule_id,
            repo=candidate.repo,
            path=rel_path,
            line=line,
            bucket=candidate.bucket,
            registry_verdict=candidate.registry_verdict,
            llm_verdict="unsure",
            label_token="C",
            model=manifest.model_id,
            quant=manifest.quantization,
            runtime=manifest.runtime,
            prompt_version=manifest.prompt_version,
            input_sha256=sha256_text(build_prompt(
                rule_meta,
                dialect=dialect,
                line=line,
                span=span,
                message=message,
                sql=packed_sql,
            )),
            model_sha256=model_sha256,
            cache_key=key,
            created_at=utc_now_iso(),
            logprobs={"A": -100.0, "B": -100.0, "C": 0.0},
            abstention_reason="unsure_due_to_context_limit",
            context_truncated=truncated,
            rule_description_sha=rule_description_sha,
            sql_sha=sql_sha,
            finding_span=span,
            runtime_version=runtime_ver,
            message=message,
            dialect=dialect,
        )

    prompt = build_prompt(
        rule_meta,
        dialect=dialect,
        line=line,
        span=span,
        message=message,
        sql=packed_sql,
    )
    if judge is None:
        raise SystemExit("judge backend required for uncached findings")

    letter, logprobs = judge.judge(prompt, max_tokens=manifest.max_new_tokens)
    llm_verdict, abstention_reason = decide_verdict(
        logprobs["A"],
        logprobs["B"],
        letter,
    )
    if truncated and llm_verdict in {"tp", "fp"} and abstention_reason is None:
        llm_verdict = "unsure"
        abstention_reason = "context_truncated"

    return JudgeRecord(
        finding_id=finding_id,
        rule_id=rule_id,
        repo=candidate.repo,
        path=rel_path,
        line=line,
        bucket=candidate.bucket,
        registry_verdict=candidate.registry_verdict,
        llm_verdict=llm_verdict,
        label_token=letter,
        model=manifest.model_id,
        quant=manifest.quantization,
        runtime=manifest.runtime,
        prompt_version=manifest.prompt_version,
        input_sha256=sha256_text(prompt),
        model_sha256=model_sha256,
        cache_key=key,
        created_at=utc_now_iso(),
        logprobs=logprobs,
        abstention_reason=abstention_reason,
        context_truncated=truncated,
        rule_description_sha=rule_description_sha,
        sql_sha=sql_sha,
        finding_span=span,
        runtime_version=runtime_ver,
        message=message,
        dialect=dialect,
    )


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
    parser.add_argument("--dry-run", action="store_true", help="Enumerate candidates only")
    args = parser.parse_args()

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
    print(f"Collected {len(candidates)} findings; capped to {len(capped)} ({args.cap}/bucket)")

    if args.dry_run:
        labeled = sum(1 for item in capped if item.registry_verdict is not None)
        print(f"Labeled buckets: {labeled}/{len(capped)}")
        return 0

    assert model_path is not None
    if not model_path.exists():
        raise SystemExit(f"model file not found: {model_path}")

    model_sha256 = file_sha256(model_path)
    manifest = JudgeManifest(
        judge_name=JUDGE_NAME,
        judge_version=JUDGE_VERSION,
        model_id=args.model_id,
        model_file_sha256=model_sha256,
        quantization=args.quant,
        runtime=DEFAULT_RUNTIME,
        runtime_version=runtime_version(),
        prompt_version=PROMPT_VERSION,
        temperature=0.0,
        seed=args.seed,
        context_tokens=args.context_tokens,
        max_new_tokens=DEFAULT_MAX_NEW_TOKENS,
        cap=args.cap,
        repo=args.repo,
        sample_seed=args.seed,
    )

    existing = load_judge_records(args.out)
    cache = {record.cache_key: record for record in existing}
    rule_meta_map = load_rule_metadata()
    judge = LlamaJudge(model_path, n_ctx=manifest.context_tokens, seed=manifest.seed)

    records: list[JudgeRecord] = []
    for index, candidate in enumerate(capped, start=1):
        record = build_record(
            candidate,
            checkout=checkout,
            compiled=compiled,
            dialect=dialect,
            rule_meta_map=rule_meta_map,
            manifest=manifest,
            model_sha256=model_sha256,
            judge=judge,
            cache=cache,
        )
        records.append(record)
        if index % 10 == 0 or index == len(capped):
            print(f"judged {index}/{len(capped)}")

    write_judge_records(records, args.out)
    write_manifest(manifest, args.manifest_out)
    print(f"wrote {args.out}")
    print(f"wrote {args.manifest_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

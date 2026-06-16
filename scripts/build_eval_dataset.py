#!/usr/bin/env python3
"""Seed or refresh the frozen eval_labels.toml classification dataset."""

from __future__ import annotations

import argparse
import random
import sys
import tomllib
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from eval_lib import (  # noqa: E402
    BUCKET_PATH_PREFIX,
    CORPUS_MANIFEST,
    CORPUS_REPO,
    CORPUS_SHA,
    DEFAULT_LABELS,
    FP_REGISTRY,
    EvalLabel,
    label_sort_key,
    load_repos,
    write_eval_labels,
)

INFRASTRUCTURE_RULES = {
    "SQLCOST023",
    "SQLCOST024",
    "SQLCOST025",
    "SQLCOST026",
    "SQLCOST027",
}


def seed_corpus_labels() -> list[EvalLabel]:
    data = tomllib.loads(CORPUS_MANIFEST.read_text(encoding="utf-8"))
    labels: list[EvalLabel] = []
    for case in data.get("case", []):
        case_path = case["path"]
        for rule in case.get("expect_rules", []):
            labels.append(
                EvalLabel(
                    repo=CORPUS_REPO,
                    sha=CORPUS_SHA,
                    rule=rule,
                    path=case_path,
                    y_true=1,
                    split="corpus",
                    source="seed:corpus",
                    notes=f"expect_rules from case {case['name']}",
                )
            )
        for rule in case.get("forbid_rules", []):
            labels.append(
                EvalLabel(
                    repo=CORPUS_REPO,
                    sha=CORPUS_SHA,
                    rule=rule,
                    path=case_path,
                    y_true=0,
                    split="corpus",
                    source="seed:corpus",
                    notes=f"forbid_rules from case {case['name']}",
                )
            )
    return labels


def seed_fp_registry_labels() -> list[EvalLabel]:
    data = tomllib.loads(FP_REGISTRY.read_text(encoding="utf-8"))
    repos = load_repos()
    seen: set[tuple[str, str, str, str]] = set()
    labels: list[EvalLabel] = []
    for entry in data.get("finding", []):
        rule = entry.get("rule")
        bucket = entry.get("bucket")
        verdict = entry.get("verdict")
        repo_name = entry.get("repo", "spellbook")
        if not rule or not bucket or verdict not in {"tp", "fp"}:
            continue
        if rule in INFRASTRUCTURE_RULES:
            continue
        repo = repos.get(repo_name)
        if repo is None:
            continue
        sha = str(repo["commit"])
        path = f"{BUCKET_PATH_PREFIX}{bucket}"
        key = (repo_name, sha, rule, path)
        if key in seen:
            continue
        seen.add(key)
        labels.append(
            EvalLabel(
                repo=repo_name,
                sha=sha,
                rule=rule,
                path=path,
                y_true=1 if verdict == "tp" else 0,
                split="real",
                source="seed:fp_registry",
                notes=str(entry.get("notes", "")),
            )
        )
    return labels


def collect_model_paths(checkout: Path, scan_paths: list[str]) -> list[str]:
    paths: list[str] = []
    for scan_path in scan_paths:
        root = checkout / scan_path
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if path.suffix.lower() in {".sql", ".py"} and path.is_file():
                rel = normalize_repo_path(checkout, path)
                paths.append(rel)
    return sorted(set(paths))


def normalize_repo_path(checkout: Path, path: Path) -> str:
    return path.relative_to(checkout).as_posix()


def seed_negative_samples(
    *,
    repo_name: str,
    cache: Path,
    sample_size: int,
    seed: int,
    rules: list[str],
) -> list[EvalLabel]:
    from bucket_rule_diagnostics import run_scan  # noqa: E402
    from costguard_tooling import repo_by_name  # noqa: E402

    repo = repo_by_name(repo_name)
    checkout = cache / repo_name
    manifest = checkout / "target" / "manifest.json"
    if not manifest.exists():
        return []

    payload = run_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        manifest=manifest,
    )
    fired_by_path: dict[str, set[str]] = defaultdict(set)
    for diagnostic in payload.get("diagnostics", []):
        rule_id = diagnostic.get("rule_id")
        rel_path = str(diagnostic.get("path", "")).replace("\\", "/")
        if rule_id and rel_path:
            fired_by_path[rel_path].add(rule_id)

    model_paths = collect_model_paths(checkout, repo.get("scan_paths", ["."]))
    candidates: list[tuple[str, str]] = []
    for rel_path in model_paths:
        fired = fired_by_path.get(rel_path, set())
        for rule in rules:
            if rule not in fired:
                candidates.append((rel_path, rule))

    if not candidates:
        return []

    rng = random.Random(seed)
    chosen = rng.sample(candidates, min(sample_size, len(candidates)))
    population = len(candidates)
    weight = population / len(chosen)
    sha = str(repo["commit"])
    return [
        EvalLabel(
            repo=repo_name,
            sha=sha,
            rule=rule,
            path=rel_path,
            y_true=None,
            split="real",
            source="seed:negative_sample",
            weight=weight,
            notes="provisional TN stub; needs human y_true",
            pending=True,
        )
        for rel_path, rule in chosen
    ]


def merge_labels(*groups: list[EvalLabel]) -> list[EvalLabel]:
    merged: dict[tuple[str, ...], EvalLabel] = {}
    for group in groups:
        for label in group:
            key = (label.repo, label.sha, label.rule, label.path, label.split)
            merged[key] = label
    return sorted(merged.values(), key=label_sort_key)


def build_dataset(
    *,
    sample_negatives: int,
    negative_seed: int,
    negative_repo: str,
    rules: list[str] | None,
) -> list[EvalLabel]:
    groups = [seed_corpus_labels(), seed_fp_registry_labels()]
    if sample_negatives > 0:
        rule_list = rules or [
            f"SQLCOST{idx:03d}" for idx in range(1, 23)
        ] + [f"SQLCOST{idx:03d}" for idx in range(28, 45)]
        groups.append(
            seed_negative_samples(
                repo_name=negative_repo,
                cache=Path.home() / ".cache/costguard/benchmarks",
                sample_size=sample_negatives,
                seed=negative_seed,
                rules=rule_list,
            )
        )
    return merge_labels(*groups)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--write",
        action="store_true",
        help="Write tests/benchmarks/eval_labels.toml",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=DEFAULT_LABELS,
        help="Output path for --write",
    )
    parser.add_argument(
        "--sample-negatives",
        type=int,
        default=0,
        help="Sample non-fired (path, rule) pairs from cached external repo",
    )
    parser.add_argument("--negative-seed", type=int, default=42)
    parser.add_argument("--negative-repo", default="spellbook")
    parser.add_argument("--json-out", type=Path, default=None)
    args = parser.parse_args()

    labels = build_dataset(
        sample_negatives=args.sample_negatives,
        negative_seed=args.negative_seed,
        negative_repo=args.negative_repo,
        rules=None,
    )
    corpus = sum(1 for label in labels if label.split == "corpus")
    real = sum(1 for label in labels if label.split == "real")
    pending = sum(1 for label in labels if label.pending)
    print(f"Built {len(labels)} labels (corpus={corpus}, real={real}, pending={pending})")

    if args.write:
        write_eval_labels(labels, args.out)
        print(f"wrote {args.out}")

    if args.json_out is not None:
        import json

        payload = {
            "total": len(labels),
            "corpus": corpus,
            "real": real,
            "pending": pending,
        }
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {args.json_out}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

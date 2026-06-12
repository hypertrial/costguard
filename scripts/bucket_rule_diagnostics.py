#!/usr/bin/env python3
"""Bucket Costguard rule diagnostics for external-repo triage."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from collections.abc import Callable
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import repo_by_name, run_costguard_scan  # noqa: E402

CROSS_JOIN_RE = re.compile(r"(?i)\bcross\s+join\b")
CROSS_JOIN_UNNEST_RE = re.compile(r"(?i)\bcross\s+join\s+(?:unnest|table)\s*\(")
FROM_COMMA_RE = re.compile(r"(?i)\bfrom\b")
JOIN_ON_RE = re.compile(r"(?i)\bon\b")
LOWER_TRIM_RE = re.compile(r"(?i)\b(?:lower|upper|trim|ltrim|rtrim)\s*\(")
CAST_RE = re.compile(r"(?i)\bcast\s*\(")
COALESCE_RE = re.compile(r"(?i)\bcoalesce\s*\(")
HASH_RE = re.compile(r"(?i)\b(?:keccak|sha256|md5|hash)\s*\(")
DATE_TRUNC_RE = re.compile(r"(?i)\bdate_trunc\s*\(")
BLOCK_TIME_RE = re.compile(r"(?i)\b(?:block_time|evt_block_time|evt_block_date|block_date)\b")
SOURCE_RE = re.compile(r"(?i)\bsource\s*\(")
IS_INCREMENTAL_RE = re.compile(r"(?i)is_incremental\s*\(")
DISTINCT_RE = re.compile(r"(?i)\bselect\s+distinct\b")
GROUP_BY_RE = re.compile(r"(?i)\bgroup\s+by\b")
UNION_RE = re.compile(r"(?i)\bunion\b(?!\s+all\b)")
WINDOW_OVER_RE = re.compile(r"(?i)\bover\s*\(\s*\)")
JSON_EXTRACT_RE = re.compile(r"(?i)\bjson_(?:extract|parse|value)\w*\s*\(")
REGEX_CALL_RE = re.compile(r"(?i)\b(?:regexp_extract|regexp_replace|regexp_like|rlike)\s*\(")
COUNT_DISTINCT_RE = re.compile(r"(?i)\bcount\s*\(\s*distinct\b")
CTE_REF_RE = re.compile(r"(?i)\bwith\s+\w+\s+as\s*\(")
NON_EQ_JOIN_RE = re.compile(r"(?i)\bon\b[^=]{0,120}(?:<>|!=|>|<|\blike\b|\brlike\b)")
EQ_JOIN_RE = re.compile(r"(?i)\bon\b[^=]{0,120}=")
LEADING_WILDCARD_LIKE_RE = re.compile(r"(?i)\b(?:like|ilike)\s+'%")
OR_PREDICATE_RE = re.compile(r"(?i)\bwhere\b[^;]*\bor\b")
CORRELATED_SUBQUERY_RE = re.compile(r"(?i)\bwhere\b[^;]*\(\s*select\b")
SCALAR_SUBQUERY_RE = re.compile(r"(?i)\bselect\b[^;]*\(\s*select\b[^)]+\)\s*(?:,|\bfrom\b)")
PATTERN_JOIN_RE = re.compile(r"(?i)\bon\b[^;]*(?:like|rlike|regexp_like|regexp_extract)\s*\(")


def mask_literals_and_comments(text: str) -> str:
    output: list[str] = []
    i = 0
    while i < len(text):
        if text.startswith("--", i):
            end = text.find("\n", i)
            if end == -1:
                output.append(" " * (len(text) - i))
                break
            output.append(" " * (end - i))
            output.append("\n")
            i = end + 1
            continue
        if text[i] == "'":
            j = i + 1
            while j < len(text):
                if text[j] == "'" and text[j - 1 : j + 1] != "''":
                    break
                j += 1
            output.append(" " * (j - i + 1))
            i = j + 1
            continue
        output.append(text[i])
        i += 1
    return "".join(output)


def cross_join_only_in_literals(text: str) -> bool:
    if not CROSS_JOIN_RE.search(text):
        return False
    masked = mask_literals_and_comments(text)
    return not CROSS_JOIN_RE.search(masked)


def has_top_level_comma_after_from(masked: str) -> bool:
    for match in FROM_COMMA_RE.finditer(masked):
        start = match.end()
        depth = 0
        i = start
        while i < len(masked):
            ch = masked[i]
            if ch == "(":
                depth += 1
            elif ch == ")":
                depth = max(depth - 1, 0)
            elif ch == "," and depth == 0:
                tail = masked[i + 1 : i + 80].lstrip()
                if tail and tail[0].isalpha():
                    return True
                break
            lower_tail = masked[i : i + 12].lower()
            if depth == 0 and lower_tail.startswith(
                (" where ", " group by ", " order by ", " limit ", "\nwhere ")
            ):
                break
            i += 1
    return False


def looks_like_subquery_comma_fp(masked: str) -> bool:
    trimmed = masked.strip()
    if not trimmed.lower().startswith("("):
        return False
    lower = trimmed.lower()
    select_idx = lower.find("select")
    comma_idx = lower.find(",")
    if select_idx == -1 or comma_idx == -1:
        return False
    return select_idx < comma_idx


def has_comma_join_in_table_list(tail: str) -> bool:
    depth = 0
    i = 0
    while i < len(tail):
        ch = tail[i]
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth = max(depth - 1, 0)
        elif ch == "," and depth == 0:
            rest = tail[i + 1 :].lstrip()
            if rest and (rest[0].isalpha() or rest[0] == "(" or rest[0] == "_"):
                return True
            return False
        if depth == 0:
            lower_tail = tail[i : i + 12].lower()
            if lower_tail.startswith(
                (" where ", " group by ", " order by ", " limit ", "\nwhere ")
            ):
                return False
        i += 1
    return False


def from_clause_tables_tail(masked: str) -> str | None:
    lower = masked.lower()
    patterns = [" from ", "\nfrom ", "\r\nfrom ", "\tfrom "]
    depth = 0
    last_start: int | None = None
    i = 0
    while i < len(lower):
        ch = lower[i]
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth = max(depth - 1, 0)
        elif depth == 0:
            if lower.startswith("from ", i):
                last_start = i + 5
            else:
                for pattern in patterns:
                    if lower.startswith(pattern, i):
                        last_start = i + len(pattern)
                        break
        i += 1
    if last_start is None:
        return None
    return masked[last_start:]


def classify_sqlcost012(sql: str) -> str:
    if cross_join_only_in_literals(sql):
        return "string_literal_fp"

    masked = mask_literals_and_comments(sql)
    lower = masked.lower()

    if CROSS_JOIN_UNNEST_RE.search(lower):
        return "cross_join_unnest"

    from_tail = from_clause_tables_tail(masked)
    if from_tail and looks_like_subquery_comma_fp(from_tail):
        return "subquery_comma_fp"

    if CROSS_JOIN_RE.search(lower):
        if "date_spine" in lower or "date_ranges" in lower:
            return "date_spine_cross_join"
        return "cross_join_explicit"

    if from_tail and has_comma_join_in_table_list(from_tail):
        if "group by" in lower and "from" in lower:
            return "group_by_comma_fp"
        return "comma_join"

    return "other"


def classify_sqlcost017(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if re.search(r"(?i)lower\s*\([^)]+\)\s*=\s*lower\s*\(", masked):
        return "symmetric_normalize"
    if DATE_TRUNC_RE.search(masked) and JOIN_ON_RE.search(masked):
        return "date_trunc_join"
    if CAST_RE.search(masked):
        return "cast_on_key"
    if COALESCE_RE.search(masked) and JOIN_ON_RE.search(masked):
        return "coalesce_key"
    if HASH_RE.search(masked) and JOIN_ON_RE.search(masked):
        return "hash_bytes"
    if LOWER_TRIM_RE.search(masked) and JOIN_ON_RE.search(masked):
        return "lower_trim"
    return "other"


def classify_sqlcost016(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if DATE_TRUNC_RE.search(masked) and BLOCK_TIME_RE.search(masked):
        return "date_trunc_filter"
    if CAST_RE.search(masked) and BLOCK_TIME_RE.search(masked):
        return "cast_partition"
    if BLOCK_TIME_RE.search(masked):
        return "function_on_block_time"
    return "other"


def classify_sqlcost019(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if not IS_INCREMENTAL_RE.search(masked) or not SOURCE_RE.search(masked):
        return "other"
    if BLOCK_TIME_RE.search(masked):
        if "where" in masked and BLOCK_TIME_RE.search(masked.split("where", 1)[-1]):
            return "block_time_in_incremental"
        if JOIN_ON_RE.search(masked) and BLOCK_TIME_RE.search(masked):
            return "block_time_in_source_scope"
    if "with " in masked and SOURCE_RE.search(masked):
        return "macro_wrapped"
    return "no_where_on_source"


def classify_sqlcost005(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if not IS_INCREMENTAL_RE.search(masked):
        return "other"
    if "incremental_predicate(" in masked:
        return "config_predicate"
    if BLOCK_TIME_RE.search(masked):
        return "block_time_present"
    if DATE_TRUNC_RE.search(masked):
        return "date_trunc_present"
    if "not in (select" in masked or "except select" in masked:
        return "anti_join_pattern"
    return "missing_predicate"


def classify_sqlcost002(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    matches = len(JSON_EXTRACT_RE.findall(masked))
    if matches >= 2:
        return "repeated_json"
    return "other"


def classify_sqlcost003(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if len(REGEX_CALL_RE.findall(masked)) >= 2:
        return "repeated_regex"
    return "other"


def classify_sqlcost006(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if NON_EQ_JOIN_RE.search(masked):
        return "non_equality_join"
    if EQ_JOIN_RE.search(masked):
        return "equality_join"
    return "other"


def classify_sqlcost008(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if not DISTINCT_RE.search(masked):
        return "other"
    if GROUP_BY_RE.search(masked):
        return "distinct_with_group_by"
    return "blind_distinct"


def classify_sqlcost013(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if WINDOW_OVER_RE.search(masked):
        return "empty_partition_by"
    return "other"


def classify_sqlcost014(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if not CTE_REF_RE.search(masked):
        return "other"
    cte_names = re.findall(r"(?i)\bwith\s+(\w+)\s+as\s*\(", masked)
    for name in cte_names:
        if len(re.findall(rf"\b{re.escape(name.lower())}\b", masked)) > 1:
            return "repeated_cte_ref"
    return "other"


def classify_sqlcost015(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if JSON_EXTRACT_RE.search(masked):
        return "json_expression"
    if REGEX_CALL_RE.search(masked):
        return "regex_expression"
    if LOWER_TRIM_RE.search(masked):
        return "normalization_expression"
    return "other"


def classify_sqlcost018(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if UNION_RE.search(masked):
        return "union_without_all"
    return "other"


def classify_sqlcost020(sql: str) -> str:
    masked = mask_literals_and_comments(sql).lower()
    if COUNT_DISTINCT_RE.search(masked):
        return "count_distinct"
    return "other"


CLASSIFIERS: dict[str, Callable[[str], str]] = {
    "SQLCOST002": classify_sqlcost002,
    "SQLCOST003": classify_sqlcost003,
    "SQLCOST005": classify_sqlcost005,
    "SQLCOST006": classify_sqlcost006,
    "SQLCOST008": classify_sqlcost008,
    "SQLCOST012": classify_sqlcost012,
    "SQLCOST013": classify_sqlcost013,
    "SQLCOST014": classify_sqlcost014,
    "SQLCOST015": classify_sqlcost015,
    "SQLCOST016": classify_sqlcost016,
    "SQLCOST017": classify_sqlcost017,
    "SQLCOST018": classify_sqlcost018,
    "SQLCOST019": classify_sqlcost019,
    "SQLCOST020": classify_sqlcost020,
}


def load_repo(name: str) -> dict[str, Any]:
    return repo_by_name(name)


def run_scan(
    checkout: Path,
    *,
    warehouse: str,
    scan_paths: list[str],
    manifest: Path,
) -> dict[str, Any]:
    payload, _exit_code = run_costguard_scan(
        checkout,
        warehouse=warehouse,
        scan_paths=scan_paths,
        fail_on="critical",
        manifest=manifest,
    )
    return payload


def load_manifest_sql(manifest_path: Path) -> dict[str, str]:
    payload = json.loads(manifest_path.read_text(encoding="utf-8"))
    compiled: dict[str, str] = {}
    for node in payload.get("nodes", {}).values():
        if node.get("resource_type") != "model":
            continue
        path = node.get("original_file_path") or node.get("path")
        code = node.get("compiled_code")
        if path and code:
            compiled[path.replace("\\", "/")] = code
    return compiled


def load_audit_by_path(audit_json: Path) -> dict[str, dict[str, Any]]:
    payload = json.loads(audit_json.read_text(encoding="utf-8"))
    by_path: dict[str, dict[str, Any]] = {}
    for item in payload.get("items", []):
        path = str(item.get("original_file_path", "")).replace("\\", "/")
        if path:
            by_path[path] = item
    return by_path


def parse_status_by_path(scan_payload: dict[str, Any]) -> dict[str, dict[str, Any]]:
    by_path: dict[str, dict[str, Any]] = {}
    for entry in scan_payload.get("files", []):
        path = str(entry.get("path", "")).replace("\\", "/")
        if path:
            by_path[path] = entry
    return by_path


def read_sql_for_diagnostic(
    checkout: Path,
    diagnostic: dict[str, Any],
    compiled_by_path: dict[str, str],
) -> str:
    rel_path = diagnostic.get("path", "").replace("\\", "/")
    if rel_path in compiled_by_path:
        return compiled_by_path[rel_path]
    file_path = checkout / rel_path
    if file_path.exists():
        return file_path.read_text(encoding="utf-8", errors="replace")
    return ""


def bucket_diagnostics(
    diagnostics: list[dict[str, Any]],
    checkout: Path,
    compiled_by_path: dict[str, str],
    rule_id: str,
    limit: int | None,
    parse_status: dict[str, dict[str, Any]],
    audit_by_path: dict[str, dict[str, Any]],
    parse_input_filter: str | None,
) -> dict[str, Any]:
    filtered = [d for d in diagnostics if d.get("rule_id") == rule_id]
    if parse_input_filter is not None:
        filtered = [
            d
            for d in filtered
            if parse_status.get(str(d.get("path", "")).replace("\\", "/"), {}).get(
                "parse_input"
            )
            == parse_input_filter
        ]
    if limit is not None:
        filtered = filtered[:limit]

    classifier = CLASSIFIERS.get(rule_id, lambda _sql: "other")
    buckets: Counter[str] = Counter()
    examples: dict[str, list[dict[str, str]]] = defaultdict(list)

    for diagnostic in filtered:
        rel_path = str(diagnostic.get("path", "")).replace("\\", "/")
        sql = read_sql_for_diagnostic(checkout, diagnostic, compiled_by_path)
        bucket = classifier(sql)
        buckets[bucket] += 1
        if len(examples[bucket]) < 5:
            line = str(diagnostic.get("line", ""))
            snippet = sql[max(0, (diagnostic.get("line", 1) - 1) * 40) :][:240].replace(
                "\n", " "
            )
            example: dict[str, str] = {
                "path": rel_path,
                "line": line,
                "snippet": snippet,
            }
            parse_entry = parse_status.get(rel_path, {})
            if parse_entry:
                example["parse_input"] = str(parse_entry.get("parse_input", ""))
                example["feature_extraction_used_ast"] = str(
                    parse_entry.get("feature_extraction_used_ast", "")
                )
            audit_entry = audit_by_path.get(rel_path)
            if audit_entry:
                example["error_signature"] = str(audit_entry.get("error_signature", ""))
            examples[bucket].append(example)

    return {
        "rule_id": rule_id,
        "total": len(filtered),
        "buckets": dict(buckets),
        "examples": dict(examples),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path.home() / ".cache" / "costguard" / "benchmarks",
        help="Benchmark cache root",
    )
    parser.add_argument(
        "--repo",
        default="spellbook",
        help="Repo name from tests/benchmarks/repos.toml",
    )
    parser.add_argument("--rule", default="SQLCOST012", help="Rule id to bucket")
    parser.add_argument("--limit", type=int, default=None, help="Max diagnostics to classify")
    parser.add_argument(
        "--parse-input-filter",
        default=None,
        help="Filter diagnostics to files with this parse_input value",
    )
    parser.add_argument(
        "--join-audit",
        type=Path,
        default=None,
        help="Optional audit_compiled_parse --json output",
    )
    parser.add_argument("--json-out", type=Path, default=None, help="Write JSON report")
    args = parser.parse_args()

    repo = load_repo(args.repo)
    checkout = args.cache / args.repo
    manifest = checkout / "target" / "manifest.json"
    if not manifest.exists():
        raise SystemExit(
            f"missing manifest at {manifest}; run benchmark_external_repo.py --repo {args.repo} first"
        )

    payload = run_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        manifest=manifest,
    )
    compiled_by_path = load_manifest_sql(manifest)
    audit_by_path = load_audit_by_path(args.join_audit) if args.join_audit else {}
    parse_status = parse_status_by_path(payload)
    report = bucket_diagnostics(
        payload.get("diagnostics", []),
        checkout,
        compiled_by_path,
        args.rule,
        args.limit,
        parse_status,
        audit_by_path,
        args.parse_input_filter,
    )

    print(f"Rule {args.rule}: {report['total']} diagnostics")
    for bucket, count in sorted(report["buckets"].items(), key=lambda item: -item[1]):
        print(f"  {bucket}: {count}")
        for example in report["examples"].get(bucket, []):
            print(f"    - {example['path']}:{example['line']} {example['snippet'][:100]}")

    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {args.json_out}")


if __name__ == "__main__":
    main()

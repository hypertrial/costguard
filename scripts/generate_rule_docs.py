#!/usr/bin/env python3
"""Generate the mdBook rule catalog from `costguard rules --format json`."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "docs" / "book" / "rules" / "index.md"
GENERATED_START = "<!-- generated:rules:start -->"
GENERATED_END = "<!-- generated:rules:end -->"


def costguard_binary() -> Path:
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target_dir / "debug" / "costguard"
    if binary.exists():
        return binary
    build = subprocess.run(
        ["cargo", "build", "-q", "-p", "costguard-cli"],
        cwd=ROOT,
        check=False,
    )
    if build.returncode != 0:
        raise SystemExit("failed to build costguard-cli")
    if not binary.exists():
        raise SystemExit(f"costguard binary not found at {binary}")
    return binary


def fetch_rules() -> list[dict[str, str]]:
    proc = subprocess.run(
        [str(costguard_binary()), "rules", "--format", "json"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise SystemExit(proc.stderr.strip() or "costguard rules failed")
    return json.loads(proc.stdout)


def render_table(rules: list[dict[str, str]]) -> str:
    lines = [
        "| Severity | Rule | Name | Guide |",
        "| --- | --- | --- | --- |",
    ]
    for rule in rules:
        rule_id = rule["id"]
        severity = rule.get("severity", "unknown")
        name = rule.get("name", "")
        guide = f"[{rule_id}](../../rules/{rule_id}.md)"
        lines.append(f"| {severity} | `{rule_id}` | {name} | {guide} |")
    return "\n".join(lines)


def render_details(rules: list[dict[str, str]]) -> str:
    blocks: list[str] = []
    for rule in rules:
        rule_id = rule["id"]
        blocks.append(f"### `{rule_id}` — {rule.get('name', '')}")
        blocks.append("")
        blocks.append(f"**Severity:** {rule.get('severity', 'unknown')}")
        blocks.append("")
        blocks.append(rule.get("description", ""))
        blocks.append("")
        blocks.append(f"Fix guidance: [{rule_id}.md](../../rules/{rule_id}.md)")
        blocks.append("")
    return "\n".join(blocks)


def build_document(rules: list[dict[str, str]]) -> str:
    generated = "\n".join(
        [
            render_table(rules),
            "",
            "## Descriptions",
            "",
            render_details(rules),
        ]
    )
    return "\n".join(
        [
            "# Rule catalog",
            "",
            "Generated from `costguard rules --format json`. Regenerate with:",
            "",
            "```bash",
            "python3 scripts/generate_rule_docs.py",
            "```",
            "",
            GENERATED_START,
            generated.rstrip(),
            GENERATED_END,
            "",
        ]
    )


def read_existing_generated(path: Path) -> str | None:
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8")
    start = text.find(GENERATED_START)
    end = text.find(GENERATED_END)
    if start == -1 or end == -1:
        return None
    return text[start : end + len(GENERATED_END)]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Exit 1 when docs/book/rules/index.md is stale",
    )
    args = parser.parse_args()

    rules = fetch_rules()
    document = build_document(rules)
    generated = document[document.find(GENERATED_START) : document.find(GENERATED_END) + len(GENERATED_END)]

    if args.check:
        existing = read_existing_generated(OUTPUT)
        if existing != generated:
            print(f"stale rule catalog: {OUTPUT}", file=sys.stderr)
            raise SystemExit(1)
        print(f"rule catalog up to date: {OUTPUT}")
        return

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(document, encoding="utf-8")
    print(f"wrote {OUTPUT}")


if __name__ == "__main__":
    main()

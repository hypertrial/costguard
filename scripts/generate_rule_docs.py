#!/usr/bin/env python3
"""Generate the mdBook rule catalog from `costguard rules --format json`."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
OUTPUT = ROOT / "docs" / "book" / "rules" / "index.md"
RULE_GUIDES = ROOT / "docs" / "rules"
PRECISION_TIERS = ROOT / "tests" / "benchmarks" / "precision_tiers.toml"
GENERATED_START = "<!-- generated:rules:start -->"
GENERATED_END = "<!-- generated:rules:end -->"


from costguard_tooling import costguard_binary


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


def load_precision_tiers() -> dict[str, str]:
    if not PRECISION_TIERS.exists():
        return {}
    data = tomllib.loads(PRECISION_TIERS.read_text(encoding="utf-8"))
    return {entry["id"]: entry["tier"] for entry in data.get("rule", [])}


def render_table(rules: list[dict[str, str]], tiers: dict[str, str]) -> str:
    lines = [
        "| Severity | Rule | Name | Measured precision | Guide |",
        "| --- | --- | --- | --- | --- |",
    ]
    for rule in rules:
        rule_id = rule["id"]
        severity = rule.get("severity", "unknown")
        name = rule.get("name", "")
        tier = tiers.get(rule_id, "—")
        guide = f"[{rule_id}](../../rules/{rule_id}.md)"
        lines.append(f"| {severity} | `{rule_id}` | {name} | {tier} | {guide} |")
    return "\n".join(lines)


def render_details(rules: list[dict[str, str]], tiers: dict[str, str]) -> str:
    blocks: list[str] = []
    for rule in rules:
        rule_id = rule["id"]
        blocks.append(f"### `{rule_id}` — {rule.get('name', '')}")
        blocks.append("")
        blocks.append(f"**Severity:** {rule.get('severity', 'unknown')}")
        if tier := tiers.get(rule_id):
            blocks.append(f"**Measured precision tier:** {tier}")
        blocks.append("")
        blocks.append(rule.get("description", ""))
        blocks.append("")
        blocks.append(f"Fix guidance: [{rule_id}.md](../../rules/{rule_id}.md)")
        blocks.append("")
    return "\n".join(blocks)


def build_document(rules: list[dict[str, str]], tiers: dict[str, str]) -> str:
    generated = "\n".join(
        [
            render_table(rules, tiers),
            "",
            "## Descriptions",
            "",
            render_details(rules, tiers),
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


def validate_rule_guides(rules: list[dict[str, str]]) -> list[str]:
    errors: list[str] = []
    for rule in rules:
        rule_id = rule["id"]
        guide = RULE_GUIDES / f"{rule_id}.md"
        if not guide.exists():
            errors.append(f"missing per-rule guide: docs/rules/{rule_id}.md")
    extra = sorted(
        path.stem
        for path in RULE_GUIDES.glob("SQLCOST*.md")
        if path.stem not in {rule["id"] for rule in rules}
    )
    errors.extend(f"orphan per-rule guide: docs/rules/{rule_id}.md" for rule_id in extra)
    return errors


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Exit 1 when docs/book/rules/index.md is stale",
    )
    args = parser.parse_args()

    rules = fetch_rules()
    tiers = load_precision_tiers()
    document = build_document(rules, tiers)
    generated = document[document.find(GENERATED_START) : document.find(GENERATED_END) + len(GENERATED_END)]

    if args.check:
        guide_errors = validate_rule_guides(rules)
        if guide_errors:
            for error in guide_errors:
                print(error, file=sys.stderr)
            raise SystemExit(1)
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

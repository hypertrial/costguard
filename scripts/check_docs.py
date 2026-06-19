#!/usr/bin/env python3
"""Validate Markdown links using only the Python standard library."""

from __future__ import annotations

import argparse
import re
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LINK_RE = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")
HEADING_RE = re.compile(r"^#{1,6}\s+(.+?)\s*$", re.MULTILINE)
VERSION_CLAIM_RE = re.compile(r"Version `(\d+\.\d+\.\d+)`")
WORKSPACE_VERSION_RE = re.compile(
    r'^\[workspace\.package\]\s*\n(?:.*\n)*?version\s*=\s*"([^"]+)"',
    re.MULTILINE,
)
OUTPUT_SCHEMA_VERSION_RE = re.compile(
    r"const\s+OUTPUT_SCHEMA_VERSION:\s*u8\s*=\s*(\d+);"
)
JSON_OUTPUT_SCHEMA_CLAIM_RE = re.compile(
    r"(?i)(?:json\s+)?(?:output\s+)?schema(?:\s+version|\s+v|:\s*)(\d+)"
)
BASELINE_SCHEMA_CLAIM_RE = re.compile(r"(?i)baseline schema v(\d+)")
POLICY_SCHEMA_CLAIM_RE = re.compile(r"(?i)policy schema v(\d+)")
RULE_COUNT_CLAIM_RE = re.compile(r"(\d+)\s+SQLCOST rules")
PUBLIC_VERSION_PIN_RE = re.compile(
    r"(?P<context>@|--tag\s+|sh\s+-s\s+--\s+|(?:COSTGUARD_)?VERSION\s*[:=]\s*[\"']?|rev:\s*)v(?P<version>\d+\.\d+\.\d+)"
)
RULE_GUIDES_DIR = ROOT / "docs" / "rules"


def slug(value: str) -> str:
    value = re.sub(r"[`*_]", "", value.strip().lower())
    value = re.sub(r"[^a-z0-9 _-]", "", value)
    return re.sub(r"[ _]+", "-", value)


def markdown_files() -> list[Path]:
    paths = [ROOT / "README.md"]
    paths.extend(sorted((ROOT / "docs").rglob("*.md")))
    for name in ["CHANGELOG.md", "SECURITY.md", "SUPPORT.md", "CONTRIBUTING.md"]:
        path = ROOT / name
        if path.exists():
            paths.append(path)
    return paths


def check_internal(source: Path, target: str) -> str | None:
    clean = target.split(maxsplit=1)[0].strip("<>")
    if not clean or clean.startswith(("http://", "https://", "mailto:")):
        return None
    path_part, _, anchor = clean.partition("#")
    destination = source if not path_part else (source.parent / path_part).resolve()
    if not destination.exists():
        return f"{source.relative_to(ROOT)}: missing link target {target}"
    if anchor and destination.suffix.lower() == ".md":
        headings = {
            slug(match.group(1))
            for match in HEADING_RE.finditer(destination.read_text(encoding="utf-8"))
        }
        if anchor.lower() not in headings:
            return f"{source.relative_to(ROOT)}: missing anchor #{anchor} in {destination.relative_to(ROOT)}"
    return None


def check_external(url: str, retries: int) -> str | None:
    request = urllib.request.Request(url, headers={"User-Agent": "costguard-doc-check/1.0"})
    for attempt in range(retries):
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                if response.status < 400:
                    return None
        except (urllib.error.URLError, TimeoutError) as exc:
            if attempt == retries - 1:
                return f"external link failed: {url}: {exc}"
            time.sleep(2**attempt)
    return f"external link failed: {url}"


def workspace_version() -> str:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = WORKSPACE_VERSION_RE.search(cargo)
    if not match:
        raise RuntimeError("unable to read [workspace.package].version from Cargo.toml")
    return match.group(1)


def output_schema_version() -> int:
    source = (ROOT / "crates/costguard-output/src/lib.rs").read_text(encoding="utf-8")
    match = OUTPUT_SCHEMA_VERSION_RE.search(source)
    if not match:
        raise RuntimeError("unable to read OUTPUT_SCHEMA_VERSION from costguard-output")
    return int(match.group(1))


def check_version_claims() -> list[str]:
    current = workspace_version()
    errors: list[str] = []
    for source in markdown_files():
        text = source.read_text(encoding="utf-8")
        for match in VERSION_CLAIM_RE.finditer(text):
            claimed = match.group(1)
            if claimed != current:
                rel = source.relative_to(ROOT)
                errors.append(
                    f"{rel}: Version `{claimed}` does not match workspace version {current}"
                )
    return errors


def check_public_version_pins(
    files: list[Path] | None = None,
    current: str | None = None,
) -> list[str]:
    current = current or workspace_version()
    errors: list[str] = []
    for source in files or markdown_files():
        text = source.read_text(encoding="utf-8")
        for line_number, line in enumerate(text.splitlines(), start=1):
            for match in PUBLIC_VERSION_PIN_RE.finditer(line):
                claimed = match.group("version")
                if claimed != current:
                    rel = source.relative_to(ROOT) if source.is_relative_to(ROOT) else source
                    errors.append(
                        f"{rel}:{line_number}: public release pin v{claimed} "
                        f"does not match workspace version v{current}"
                    )
    return errors


def rule_guide_count() -> int:
    return len(list(RULE_GUIDES_DIR.glob("SQLCOST*.md")))


def check_rule_count_claims() -> list[str]:
    current = rule_guide_count()
    errors: list[str] = []
    for source in markdown_files():
        text = source.read_text(encoding="utf-8")
        for match in RULE_COUNT_CLAIM_RE.finditer(text):
            claimed = int(match.group(1))
            if claimed != current:
                rel = source.relative_to(ROOT)
                errors.append(
                    f"{rel}: {claimed} SQLCOST rules does not match "
                    f"{current} per-rule guides in docs/rules/"
                )
    return errors


def check_output_schema_claims() -> list[str]:
    current = output_schema_version()
    errors: list[str] = []
    for source in markdown_files():
        if source.name == "CHANGELOG.md":
            continue
        text = source.read_text(encoding="utf-8")
        for match in JSON_OUTPUT_SCHEMA_CLAIM_RE.finditer(text):
            line_start = text.rfind("\n", 0, match.start()) + 1
            line_end = text.find("\n", match.end())
            if line_end == -1:
                line_end = len(text)
            line = text[line_start:line_end]
            if BASELINE_SCHEMA_CLAIM_RE.search(line) or POLICY_SCHEMA_CLAIM_RE.search(line):
                continue
            if "baseline" in line.lower() and "schema" in line.lower():
                continue
            claimed = int(match.group(1))
            if claimed != current:
                rel = source.relative_to(ROOT)
                errors.append(
                    f"{rel}: JSON output schema v{claimed} does not match OUTPUT_SCHEMA_VERSION {current}"
                )
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--external", action="store_true")
    parser.add_argument("--retries", type=int, default=3)
    args = parser.parse_args()
    errors: list[str] = []
    errors.extend(check_version_claims())
    errors.extend(check_public_version_pins())
    errors.extend(check_rule_count_claims())
    errors.extend(check_output_schema_claims())
    external_urls: set[str] = set()
    for source in markdown_files():
        text = source.read_text(encoding="utf-8")
        for match in LINK_RE.finditer(text):
            target = match.group(1).strip()
            if target.startswith(("http://", "https://")):
                external_urls.add(target)
                continue
            error = check_internal(source, target)
            if error:
                errors.append(error)
    if args.external:
        for url in sorted(external_urls):
            error = check_external(url, args.retries)
            if error:
                errors.append(error)
    if errors:
        raise SystemExit("documentation link errors:\n" + "\n".join(errors))
    print(
        f"documentation links valid ({len(markdown_files())} files"
        f"{f', {len(external_urls)} external URLs' if args.external else ''})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

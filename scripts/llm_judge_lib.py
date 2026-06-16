#!/usr/bin/env python3
"""Shared helpers for local LLM-as-judge inter-rater reliability."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import tomllib
from dataclasses import asdict, dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LABELS_JSONL = ROOT / "tests" / "benchmarks" / "llm_judge_labels.jsonl"
DEFAULT_MANIFEST = ROOT / "tests" / "benchmarks" / "llm_judge_manifest.toml"
DEFAULT_IRR_REPORT = ROOT / "tests" / "benchmarks" / "irr_report.json"
RULE_GUIDES = ROOT / "docs" / "rules"

PROMPT_VERSION = "irr_judge_v2"
PROMPT_VERSION_GROUPED = "irr_judge_v2+grouped"
MODE_PREFIX = "prefix"
MODE_GROUPED = "grouped"
JUDGE_NAME = "costguard-local-llm-judge"
JUDGE_VERSION = "v1"
DEFAULT_MODEL_ID = "Qwen3-30B-A3B-Instruct-2507"
DEFAULT_QUANT = "Q4_K_M"
DEFAULT_RUNTIME = "llama.cpp"
DEFAULT_SEED = 3407
DEFAULT_CONTEXT_TOKENS = 32768
DEFAULT_N_BATCH = 2048
DEFAULT_N_UBATCH = 512
DEFAULT_SQL_TOKEN_TARGET = 8000
DEFAULT_MAX_NEW_TOKENS = 1
DEFAULT_CAP = 50
DEFAULT_FLASH_ATTN = True
DEFAULT_SPAN_LINES = 40
DEFAULT_KV_CACHE_CAPACITY = 2

LABEL_TOKEN_TO_VERDICT = {"A": "tp", "B": "fp", "C": "unsure"}

# ponytail: one-token GBNF; upgrade path is JSON-schema if we add fields
LABEL_GRAMMAR = r"""
root ::= letter
letter ::= "A" | "B" | "C"
"""

SYSTEM_PROMPT = """\
You are an independent SQL performance-review adjudicator.
You are NOT evaluating whether Costguard fired correctly.
You decide whether the described rule truly applies to the SQL shown.
Static-analysis findings are frequently false positives.
Default to B (false positive) when the rule's failure condition is not clearly met in the SQL or evidence is insufficient.
Use C (unsure) only when the SQL needed to judge is genuinely missing from the context.
Do not use prior labels, registry buckets, or Costguard implementation details.\
"""

DECISION_SUFFIX = """\
First identify the rule's failure condition silently, then decide:
A = true positive ONLY if the SQL clearly exhibits the rule's failure condition.
B = false positive if the finding is not applicable, is harmless, or evidence is insufficient.
C = unsure ONLY if the SQL context is genuinely insufficient to judge.

Return exactly one letter (A, B, or C). No explanation.\
"""

GROUPED_DECISION_SUFFIX = """\
For each finding below, identify the rule's failure condition silently, then decide one letter per finding.
Return a JSON array only, e.g. [{"index":0,"verdict":"A"},{"index":1,"verdict":"B"}].
A = true positive ONLY if the SQL clearly exhibits the rule's failure condition.
B = false positive if not applicable, harmless, or evidence is insufficient.
C = unsure ONLY if the SQL context is genuinely insufficient to judge.\
"""


@dataclass
class RuleMetadata:
    rule_id: str
    title: str
    description: str
    rubric: str
    severity: str = "unknown"

    @property
    def description_sha(self) -> str:
        material = f"{self.title}\n{self.description}\n{self.rubric}"
        return hashlib.sha256(material.encode("utf-8")).hexdigest()


@dataclass(frozen=True)
class FindingPromptInput:
    index: int
    rule_meta: RuleMetadata
    line: int
    span: str
    message: str


@dataclass
class JudgeRecord:
    finding_id: str
    rule_id: str
    repo: str
    path: str
    line: int
    bucket: str
    registry_verdict: str | None
    llm_verdict: str
    label_token: str
    model: str
    quant: str
    runtime: str
    prompt_version: str
    input_sha256: str
    model_sha256: str
    cache_key: str
    created_at: str
    logprobs: dict[str, float] = field(default_factory=dict)
    abstention_reason: str | None = None
    context_truncated: bool = False
    rule_description_sha: str = ""
    sql_sha: str = ""
    finding_span: str = ""
    runtime_version: str = ""
    message: str = ""
    dialect: str = ""
    mode: str = MODE_PREFIX

    def to_dict(self) -> dict[str, Any]:
        payload = asdict(self)
        if payload["registry_verdict"] is None:
            payload["registry_verdict"] = None
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> JudgeRecord:
        prompt_version = str(payload.get("prompt_version", PROMPT_VERSION))
        mode = str(payload.get("mode", mode_from_prompt_version(prompt_version)))
        return cls(
            finding_id=str(payload["finding_id"]),
            rule_id=str(payload["rule_id"]),
            repo=str(payload.get("repo", "")),
            path=str(payload.get("path", "")),
            line=int(payload.get("line", 0)),
            bucket=str(payload.get("bucket", "")),
            registry_verdict=payload.get("registry_verdict"),
            llm_verdict=str(payload["llm_verdict"]),
            label_token=str(payload.get("label_token", "")),
            model=str(payload.get("model", DEFAULT_MODEL_ID)),
            quant=str(payload.get("quant", DEFAULT_QUANT)),
            runtime=str(payload.get("runtime", DEFAULT_RUNTIME)),
            prompt_version=prompt_version,
            input_sha256=str(payload.get("input_sha256", "")),
            model_sha256=str(payload.get("model_sha256", "")),
            cache_key=str(payload.get("cache_key", "")),
            created_at=str(payload.get("created_at", "")),
            logprobs={str(k): float(v) for k, v in (payload.get("logprobs") or {}).items()},
            abstention_reason=payload.get("abstention_reason"),
            context_truncated=bool(payload.get("context_truncated", False)),
            rule_description_sha=str(payload.get("rule_description_sha", "")),
            sql_sha=str(payload.get("sql_sha", "")),
            finding_span=str(payload.get("finding_span", "")),
            runtime_version=str(payload.get("runtime_version", "")),
            message=str(payload.get("message", "")),
            dialect=str(payload.get("dialect", "")),
            mode=mode,
        )


@dataclass
class JudgeManifest:
    judge_name: str = JUDGE_NAME
    judge_version: str = JUDGE_VERSION
    model_id: str = DEFAULT_MODEL_ID
    model_file_sha256: str = ""
    quantization: str = DEFAULT_QUANT
    runtime: str = DEFAULT_RUNTIME
    runtime_version: str = ""
    prompt_version: str = PROMPT_VERSION
    temperature: float = 0.0
    seed: int = DEFAULT_SEED
    context_tokens: int = DEFAULT_CONTEXT_TOKENS
    max_new_tokens: int = DEFAULT_MAX_NEW_TOKENS
    cap: int = DEFAULT_CAP
    repo: str = "spellbook"
    sample_seed: int = DEFAULT_SEED
    n_batch: int = DEFAULT_N_BATCH
    n_ubatch: int = DEFAULT_N_UBATCH
    sql_token_target: int = DEFAULT_SQL_TOKEN_TARGET
    mode: str = MODE_PREFIX
    flash_attn: bool = DEFAULT_FLASH_ATTN

    def to_toml(self) -> str:
        lines = [
            "# Pinned LLM judge configuration for inter-rater reliability.",
            "# Regenerate labels with: python3 scripts/build_llm_judge_labels.py --model $COSTGUARD_JUDGE_GGUF",
            "",
            f'judge_name = "{self.judge_name}"',
            f'judge_version = "{self.judge_version}"',
            f'model_id = "{self.model_id}"',
            f'model_file_sha256 = "{self.model_file_sha256}"',
            f'quantization = "{self.quantization}"',
            f'runtime = "{self.runtime}"',
            f'runtime_version = "{self.runtime_version}"',
            f'prompt_version = "{self.prompt_version}"',
            f"temperature = {self.temperature}",
            f"seed = {self.seed}",
            f"context_tokens = {self.context_tokens}",
            f"max_new_tokens = {self.max_new_tokens}",
            f"cap = {self.cap}",
            f'repo = "{self.repo}"',
            f"sample_seed = {self.sample_seed}",
            f"n_batch = {self.n_batch}",
            f"n_ubatch = {self.n_ubatch}",
            f"sql_token_target = {self.sql_token_target}",
            f'mode = "{self.mode}"',
            f"flash_attn = {'true' if self.flash_attn else 'false'}",
            "",
        ]
        return "\n".join(lines)


def mode_from_prompt_version(prompt_version: str) -> str:
    if prompt_version.endswith("+grouped"):
        return MODE_GROUPED
    return MODE_PREFIX


def prompt_version_for_mode(mode: str) -> str:
    if mode == MODE_GROUPED:
        return PROMPT_VERSION_GROUPED
    return PROMPT_VERSION


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def cache_key(
    *,
    finding_id: str,
    rule_id: str,
    rule_description_sha: str,
    sql_sha: str,
    finding_span: str,
    prompt_version: str,
    model_file_sha256: str,
    runtime_version: str,
    mode: str = MODE_PREFIX,
) -> str:
    material = "|".join(
        [
            finding_id,
            rule_id,
            rule_description_sha,
            sql_sha,
            finding_span,
            prompt_version,
            model_file_sha256,
            runtime_version,
            mode,
        ]
    )
    return sha256_text(material)


def verdict_from_letter(letter: str) -> tuple[str, str | None]:
    """Return (llm_verdict, abstention_reason) from a model letter."""
    normalized = letter.strip().upper()[:1]
    if normalized == "C":
        return "unsure", "model_unsure"
    verdict = LABEL_TOKEN_TO_VERDICT.get(normalized, "unsure")
    if verdict == "unsure":
        return verdict, "model_unsure"
    return verdict, None


def messages_sha256(system: str, user: str) -> str:
    return sha256_text(f"{system}\0{user}")


def verify_gguf_chat_template(model_path: Path) -> None:
    """Assert the GGUF embeds a chat template (ChatML for Qwen3)."""
    try:
        from llama_cpp import Llama  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover - local-only dep
        raise SystemExit(
            "llama-cpp-python is required for build_llm_judge_labels.py; "
            "install with: pip install -r requirements-judge.txt"
        ) from exc
    model = Llama(model_path=str(model_path), vocab_only=True, verbose=False)
    if "tokenizer.chat_template" not in model.metadata:
        raise SystemExit(f"GGUF missing tokenizer.chat_template: {model_path}")


def fetch_rules_json() -> list[dict[str, str]]:
    from costguard_tooling import costguard_binary

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


def load_rule_guide(rule_id: str) -> str:
    guide = RULE_GUIDES / f"{rule_id}.md"
    if not guide.exists():
        return ""
    return guide.read_text(encoding="utf-8").strip()


def load_rule_metadata() -> dict[str, RuleMetadata]:
    rules = fetch_rules_json()
    metadata: dict[str, RuleMetadata] = {}
    for rule in rules:
        rule_id = rule["id"]
        metadata[rule_id] = RuleMetadata(
            rule_id=rule_id,
            title=rule.get("name", rule_id),
            description=rule.get("description", ""),
            rubric=load_rule_guide(rule_id),
            severity=rule.get("severity", "unknown"),
        )
    return metadata


def _finding_suffix_parts(
    rule_meta: RuleMetadata,
    *,
    dialect: str,
    line: int,
    span: str,
    message: str,
    index: int | None = None,
) -> list[str]:
    parts = [
        f"Rule ID: {rule_meta.rule_id}",
        f"Rule title: {rule_meta.title}",
        f"Rule description: {rule_meta.description}",
    ]
    if rule_meta.rubric:
        parts.append(f"Rule rubric:\n{rule_meta.rubric}")
    if index is not None:
        parts.append(f"Finding index: {index}")
    if dialect:
        parts.append(f"SQL dialect: {dialect}")
    if line:
        parts.append(f"Finding line: {line}")
    if span:
        parts.append(f"Finding span: {span}")
    if message:
        parts.append(f"Diagnostic message: {message}")
    return parts


def build_messages(
    rule_meta: RuleMetadata,
    *,
    dialect: str,
    line: int,
    span: str,
    message: str,
    sql: str,
) -> tuple[str, str]:
    suffix_parts = _finding_suffix_parts(
        rule_meta,
        dialect=dialect,
        line=line,
        span=span,
        message=message,
    )
    suffix_parts.append(DECISION_SUFFIX)
    suffix = "\n\n".join(suffix_parts)
    user = f"SQL:\n{sql}\n\n{suffix}"
    return SYSTEM_PROMPT, user


def build_grouped_messages(
    findings: list[FindingPromptInput],
    *,
    dialect: str,
    sql: str,
) -> tuple[str, str]:
    blocks: list[str] = []
    for item in findings:
        parts = _finding_suffix_parts(
            item.rule_meta,
            dialect=dialect,
            line=item.line,
            span=item.span,
            message=item.message,
            index=item.index,
        )
        blocks.append("\n\n".join(parts))
    findings_text = "\n\n---\n\n".join(blocks)
    user = f"SQL:\n{sql}\n\nFindings:\n{findings_text}\n\n{GROUPED_DECISION_SUFFIX}"
    return SYSTEM_PROMPT, user


def build_prompt(
    rule_meta: RuleMetadata,
    *,
    dialect: str,
    line: int,
    span: str,
    message: str,
    sql: str,
) -> str:
    system, user = build_messages(
        rule_meta,
        dialect=dialect,
        line=line,
        span=span,
        message=message,
        sql=sql,
    )
    return f"{system}\n\n{user}"


def build_grouped_prompt(
    findings: list[FindingPromptInput],
    *,
    dialect: str,
    sql: str,
) -> str:
    system, user = build_grouped_messages(findings, dialect=dialect, sql=sql)
    return f"{system}\n\n{user}"


def _estimate_char_budget(token_target: int) -> int:
    # ponytail: chars/4 heuristic; upgrade path is tokenizer-aware budget
    return max(1024, token_target * 3)


def _line_window(sql: str, line: int, radius: int) -> str:
    lines = sql.splitlines()
    if not lines:
        return sql
    idx = max(0, min(len(lines) - 1, line - 1))
    start = max(0, idx - radius)
    end = min(len(lines), idx + radius + 1)
    return "\n".join(lines[start:end])


def _extract_cte_block(sql: str, line: int) -> str:
    lines = sql.splitlines()
    if not lines:
        return ""
    idx = max(0, min(len(lines) - 1, line - 1))
    for start in range(idx, -1, -1):
        if re.search(r"(?i)\bwith\s+\w+\s+as\s*\(", lines[start]):
            return "\n".join(lines[start : idx + 1])
    for start in range(idx, -1, -1):
        if re.search(r"(?i)\bselect\b", lines[start]):
            return "\n".join(lines[start : idx + 1])
    return ""


def _referenced_cte_defs(sql: str, block: str) -> str:
    names = set(re.findall(r"(?i)\b(?:from|join)\s+(\w+)\b", block))
    if not names:
        return ""
    chunks: list[str] = []
    for name in sorted(names):
        pattern = rf"(?is)\bwith\s+{re.escape(name)}\s+as\s*\("
        match = re.search(pattern, sql)
        if match:
            chunks.append(match.group(0))
    return "\n".join(chunks)


def _union_excerpts(sql: str, lines: list[int], span_lines: int) -> str:
    parts: list[str] = []
    seen: set[str] = set()
    for line in sorted(set(lines)):
        window = _line_window(sql, line, span_lines)
        cte_block = _extract_cte_block(sql, line)
        cte_defs = _referenced_cte_defs(sql, cte_block or window)
        for part in (cte_defs, cte_block or window):
            if not part:
                continue
            digest = sha256_text(part)
            if digest in seen:
                continue
            seen.add(digest)
            parts.append(part)
    return "\n\n---\n\n".join(parts)


def pack_sql(
    sql: str,
    line: int,
    *,
    context_tokens: int = DEFAULT_CONTEXT_TOKENS,
    span_lines: int = DEFAULT_SPAN_LINES,
) -> tuple[str, bool, bool]:
    """Return (packed_sql, context_truncated, too_large_for_judge)."""
    budget = _estimate_char_budget(context_tokens)
    if len(sql) <= budget:
        return sql, False, False

    window = _line_window(sql, line, span_lines)
    cte_block = _extract_cte_block(sql, line)
    cte_defs = _referenced_cte_defs(sql, cte_block)
    packed_parts = [part for part in (cte_defs, cte_block or window) if part]
    packed = "\n\n".join(packed_parts) if packed_parts else window
    if len(packed) <= budget:
        return packed, True, False
    return packed[:budget], True, True


def pack_sql_for_file(
    sql: str,
    lines: list[int],
    *,
    sql_token_target: int = DEFAULT_SQL_TOKEN_TARGET,
    span_lines: int = DEFAULT_SPAN_LINES,
) -> tuple[str, str, bool, bool]:
    """Return (context, sql_sha, context_truncated, too_large)."""
    budget = _estimate_char_budget(sql_token_target)
    if len(sql) <= budget:
        digest = sha256_text(sql)
        return sql, digest, False, False

    packed = _union_excerpts(sql, lines, span_lines)
    digest = sha256_text(packed)
    if len(packed) <= budget:
        return packed, digest, True, False
    clipped = packed[:budget]
    return clipped, sha256_text(clipped), True, True


def parse_grouped_verdicts(text: str, n: int) -> list[str]:
    """Parse grouped JSON verdicts; missing/garbled entries default to C."""
    letters = ["C"] * n
    payload = text.strip()
    start = payload.find("[")
    end = payload.rfind("]")
    if start != -1 and end != -1 and end > start:
        payload = payload[start : end + 1]
    try:
        parsed = json.loads(payload)
    except json.JSONDecodeError:
        for match in re.finditer(r'"index"\s*:\s*(\d+)\s*,\s*"verdict"\s*:\s*"([ABC])"', text):
            idx = int(match.group(1))
            if 0 <= idx < n:
                letters[idx] = match.group(2)
        return letters
    if not isinstance(parsed, list):
        return letters
    for item in parsed:
        if not isinstance(item, dict):
            continue
        idx_raw = item.get("index")
        verdict_raw = item.get("verdict")
        if idx_raw is None or verdict_raw is None:
            continue
        try:
            idx = int(idx_raw)
        except (TypeError, ValueError):
            continue
        if not (0 <= idx < n):
            continue
        letter = str(verdict_raw).strip().upper()[:1]
        letters[idx] = letter if letter in {"A", "B", "C"} else "C"
    return letters


def finding_id_for_diagnostic(repo: str, diagnostic: dict[str, Any]) -> str:
    governance = diagnostic.get("governance") or {}
    finding_id = governance.get("finding_id") or diagnostic.get("finding_id")
    if finding_id:
        return str(finding_id)
    material = "|".join(
        [
            repo,
            str(diagnostic.get("path", "")),
            str(diagnostic.get("line", "")),
            str(diagnostic.get("rule_id", "")),
        ]
    )
    return f"cgf_{sha256_text(material)}"


def finding_span(diagnostic: dict[str, Any]) -> str:
    span = diagnostic.get("span") or {}
    start = span.get("start_line") or diagnostic.get("line")
    end = span.get("end_line") or start
    column = diagnostic.get("column", 0)
    return f"{start}:{column}-{end}"


def candidate_sort_key(candidate: Any) -> tuple[str, str, int]:
    diagnostic = candidate.diagnostic
    path = str(diagnostic.get("path", "")).replace("\\", "/")
    rule_id = str(diagnostic.get("rule_id", ""))
    line = int(diagnostic.get("line", 0) or 0)
    return (path, rule_id, line)


def load_judge_records(path: Path | None = None) -> list[JudgeRecord]:
    labels_path = path or DEFAULT_LABELS_JSONL
    if not labels_path.exists():
        return []
    records: list[JudgeRecord] = []
    for line in labels_path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        records.append(JudgeRecord.from_dict(json.loads(stripped)))
    return records


def write_judge_records(records: list[JudgeRecord], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [json.dumps(record.to_dict(), sort_keys=True) for record in records]
    path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def load_manifest(path: Path | None = None) -> JudgeManifest:
    manifest_path = path or DEFAULT_MANIFEST
    if not manifest_path.exists():
        return JudgeManifest()
    data = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    mode = str(data.get("mode", MODE_PREFIX))
    return JudgeManifest(
        judge_name=str(data.get("judge_name", JUDGE_NAME)),
        judge_version=str(data.get("judge_version", JUDGE_VERSION)),
        model_id=str(data.get("model_id", DEFAULT_MODEL_ID)),
        model_file_sha256=str(data.get("model_file_sha256", "")),
        quantization=str(data.get("quantization", DEFAULT_QUANT)),
        runtime=str(data.get("runtime", DEFAULT_RUNTIME)),
        runtime_version=str(data.get("runtime_version", "")),
        prompt_version=str(data.get("prompt_version", prompt_version_for_mode(mode))),
        temperature=float(data.get("temperature", 0.0)),
        seed=int(data.get("seed", DEFAULT_SEED)),
        context_tokens=int(data.get("context_tokens", DEFAULT_CONTEXT_TOKENS)),
        max_new_tokens=int(data.get("max_new_tokens", DEFAULT_MAX_NEW_TOKENS)),
        cap=int(data.get("cap", DEFAULT_CAP)),
        repo=str(data.get("repo", "spellbook")),
        sample_seed=int(data.get("sample_seed", DEFAULT_SEED)),
        n_batch=int(data.get("n_batch", DEFAULT_N_BATCH)),
        n_ubatch=int(data.get("n_ubatch", DEFAULT_N_UBATCH)),
        sql_token_target=int(data.get("sql_token_target", DEFAULT_SQL_TOKEN_TARGET)),
        mode=mode,
        flash_attn=bool(data.get("flash_attn", DEFAULT_FLASH_ATTN)),
    )


def write_manifest(manifest: JudgeManifest, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(manifest.to_toml(), encoding="utf-8")


def runtime_version() -> str:
    try:
        import llama_cpp  # type: ignore[import-not-found]
    except ImportError:
        return "unknown"
    return getattr(llama_cpp, "__version__", "unknown")


class LlamaJudge:
    """Lazy llama-cpp-python wrapper (import only when instantiated)."""

    def __init__(
        self,
        model_path: Path,
        *,
        n_ctx: int = DEFAULT_CONTEXT_TOKENS,
        seed: int = DEFAULT_SEED,
        n_gpu_layers: int = -1,
        n_batch: int = DEFAULT_N_BATCH,
        n_ubatch: int = DEFAULT_N_UBATCH,
        flash_attn: bool = DEFAULT_FLASH_ATTN,
    ) -> None:
        try:
            from llama_cpp import Llama  # type: ignore[import-not-found]
            from llama_cpp.llama_cache import (
                LlamaRAMCache,  # type: ignore[import-not-found]
            )
            from llama_cpp.llama_grammar import (
                LlamaGrammar,  # type: ignore[import-not-found]
            )
        except ImportError as exc:  # pragma: no cover - local-only dep
            raise SystemExit(
                "llama-cpp-python is required for build_llm_judge_labels.py; "
                "install with: pip install -r requirements-judge.txt"
            ) from exc
        self._seed = seed
        self._label_grammar = LlamaGrammar.from_string(LABEL_GRAMMAR.strip())
        self._llm = Llama(
            model_path=str(model_path),
            n_ctx=n_ctx,
            n_batch=n_batch,
            n_ubatch=n_ubatch,
            seed=seed,
            n_gpu_layers=n_gpu_layers,
            logits_all=False,
            flash_attn=flash_attn,
            verbose=False,
        )
        # ponytail: small RAM cache (~1-2 KV states); upgrade path is disk cache
        self._llm.set_cache(LlamaRAMCache(capacity_bytes=256 * 1024 * 1024))

    def judge(self, system: str, user: str, *, max_tokens: int = DEFAULT_MAX_NEW_TOKENS) -> str:
        output = self._llm.create_chat_completion(
            messages=[
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            max_tokens=max_tokens,
            temperature=0.0,
            top_p=1.0,
            seed=self._seed,
            grammar=self._label_grammar,
        )
        choice = output["choices"][0]
        message = choice.get("message") or {}
        letter = str(message.get("content", "")).strip().upper()[:1] or "C"
        return letter

    def judge_grouped(self, system: str, user: str, n: int) -> list[str]:
        max_tokens = max(32, min(512, 12 * n))
        output = self._llm.create_chat_completion(
            messages=[
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            max_tokens=max_tokens,
            temperature=0.0,
            top_p=1.0,
            seed=self._seed,
            response_format={"type": "json_object"},
        )
        choice = output["choices"][0]
        message = choice.get("message") or {}
        text = str(message.get("content", ""))
        return parse_grouped_verdicts(text, n)


def utc_now_iso() -> str:
    return datetime.now(tz=UTC).replace(microsecond=0).isoformat()

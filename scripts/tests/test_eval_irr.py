#!/usr/bin/env python3
"""Tests for llm_judge_lib.py and eval_irr.py."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

try:
    import numpy as np
except ImportError:  # pragma: no cover - CI uses .venv-eval
    np = None

ROOT = Path(__file__).resolve().parents[2]
import sys

sys.path.insert(0, str(ROOT / "scripts"))

from llm_judge_lib import (  # noqa: E402
    PROMPT_VERSION,
    JudgeManifest,
    JudgeRecord,
    RuleMetadata,
    build_prompt,
    cache_key,
    decide_verdict,
    load_judge_records,
    load_manifest,
    pack_sql,
    write_judge_records,
    write_manifest,
)

try:
    from eval_irr import build_report, validate_records  # noqa: E402
except ImportError:  # pragma: no cover - CI uses .venv-eval
    build_report = None
    validate_records = None

EVAL_DEPS = np is not None and build_report is not None


def sample_record(**overrides: object) -> JudgeRecord:
    base = {
        "finding_id": "cgf_abc",
        "rule_id": "SQLCOST012",
        "repo": "spellbook",
        "path": "models/foo.sql",
        "line": 12,
        "bucket": "cross_join_unnest",
        "registry_verdict": "fp",
        "llm_verdict": "fp",
        "label_token": "B",
        "model": "Qwen3-30B-A3B-Instruct-2507",
        "quant": "Q4_K_M",
        "runtime": "llama.cpp",
        "prompt_version": PROMPT_VERSION,
        "input_sha256": "input",
        "model_sha256": "modelsha",
        "cache_key": "",
        "created_at": "2026-06-16T00:00:00+00:00",
        "logprobs": {"A": -2.0, "B": -0.4, "C": -1.8},
        "rule_description_sha": "ruledesc",
        "sql_sha": "sqlsha",
        "finding_span": "12:1-12",
        "runtime_version": "0.3.0",
    }
    base.update(overrides)
    record = JudgeRecord.from_dict(base)
    record = JudgeRecord(
        **{
            **record.__dict__,
            "cache_key": cache_key(
                finding_id=record.finding_id,
                rule_id=record.rule_id,
                rule_description_sha=record.rule_description_sha,
                sql_sha=record.sql_sha,
                finding_span=record.finding_span,
                prompt_version=record.prompt_version,
                model_file_sha256=record.model_sha256,
                runtime_version=record.runtime_version,
            ),
        }
    )
    return record


class JudgeLibTests(unittest.TestCase):
    def test_decide_verdict_margin_abstains(self) -> None:
        verdict, reason = decide_verdict(-1.0, -1.2, "A")
        self.assertEqual(verdict, "unsure")
        self.assertEqual(reason, "logprob_margin")

    def test_decide_verdict_clear_winner(self) -> None:
        verdict, reason = decide_verdict(-0.2, -2.0, "A")
        self.assertEqual(verdict, "tp")
        self.assertIsNone(reason)

    def test_cache_key_stable(self) -> None:
        first = cache_key(
            finding_id="cgf_1",
            rule_id="SQLCOST012",
            rule_description_sha="abc",
            sql_sha="def",
            finding_span="1:1-1",
            prompt_version=PROMPT_VERSION,
            model_file_sha256="model",
            runtime_version="0.3.0",
        )
        second = cache_key(
            finding_id="cgf_1",
            rule_id="SQLCOST012",
            rule_description_sha="abc",
            sql_sha="def",
            finding_span="1:1-1",
            prompt_version=PROMPT_VERSION,
            model_file_sha256="model",
            runtime_version="0.3.0",
        )
        self.assertEqual(first, second)
        self.assertNotEqual(
            first,
            cache_key(
                finding_id="cgf_1",
                rule_id="SQLCOST012",
                rule_description_sha="changed",
                sql_sha="def",
                finding_span="1:1-1",
                prompt_version=PROMPT_VERSION,
                model_file_sha256="model",
                runtime_version="0.3.0",
            ),
        )

    def test_build_prompt_includes_rule_and_decision(self) -> None:
        meta = RuleMetadata(
            rule_id="SQLCOST012",
            title="Cross join",
            description="Detects cross joins",
            rubric="Allow UNNEST patterns.",
        )
        prompt = build_prompt(
            meta,
            dialect="trino",
            line=10,
            span="10:1-10",
            message="cross join detected",
            sql="select 1",
        )
        self.assertIn("SQLCOST012", prompt)
        self.assertIn("Verdict:", prompt)
        self.assertNotIn("cross_join_unnest", prompt)

    def test_pack_sql_truncates_large_input(self) -> None:
        sql = "\n".join(f"select {idx} from t" for idx in range(5000))
        packed, truncated, too_large = pack_sql(sql, 2500, context_tokens=256)
        self.assertTrue(truncated)
        self.assertLess(len(packed), len(sql))

    def test_jsonl_round_trip(self) -> None:
        record = sample_record()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "labels.jsonl"
            write_judge_records([record], path)
            loaded = load_judge_records(path)
        self.assertEqual(len(loaded), 1)
        self.assertEqual(loaded[0].finding_id, "cgf_abc")


class EvalIrrTests(unittest.TestCase):
    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_perfect_kappa(self) -> None:
        manifest = JudgeManifest(model_file_sha256="modelsha", prompt_version=PROMPT_VERSION)
        records = [
            sample_record(registry_verdict="fp", llm_verdict="fp"),
            sample_record(
                finding_id="cgf_def",
                registry_verdict="tp",
                llm_verdict="tp",
                label_token="A",
                logprobs={"A": -0.1, "B": -2.0, "C": -2.0},
            ),
        ]
        report = build_report(records, manifest)
        self.assertEqual(report["overall"]["kappa_binary_non_abstain"], 1.0)
        self.assertEqual(report["counts"]["scorable_non_abstain"], 2)

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_unsure_excluded_from_kappa(self) -> None:
        manifest = JudgeManifest(model_file_sha256="modelsha", prompt_version=PROMPT_VERSION)
        records = [
            sample_record(registry_verdict="fp", llm_verdict="unsure", label_token="C"),
            sample_record(
                finding_id="cgf_def",
                registry_verdict="tp",
                llm_verdict="tp",
                label_token="A",
            ),
        ]
        report = build_report(records, manifest)
        self.assertEqual(report["counts"]["scorable_non_abstain"], 1)
        self.assertIsNone(report["overall"]["kappa_binary_non_abstain"])

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_validate_rejects_mismatched_manifest(self) -> None:
        manifest = JudgeManifest(model_file_sha256="expected", prompt_version=PROMPT_VERSION)
        record = sample_record(model_sha256="other")
        errors = validate_records([record], manifest)
        self.assertTrue(errors)

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_empty_report(self) -> None:
        manifest = JudgeManifest()
        report = build_report([], manifest)
        self.assertEqual(report["counts"]["total"], 0)
        self.assertIsNone(report["overall"]["kappa_binary_non_abstain"])

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_manifest_round_trip(self) -> None:
        manifest = JudgeManifest(model_file_sha256="abc123")
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "manifest.toml"
            write_manifest(manifest, path)
            loaded = load_manifest(path)
        self.assertEqual(loaded.model_file_sha256, "abc123")


if __name__ == "__main__":
    unittest.main()

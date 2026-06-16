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

from build_llm_judge_labels import CandidateFinding, cap_candidates  # noqa: E402
from llm_judge_lib import (  # noqa: E402
    DEFAULT_FEWSHOTS,
    MODE_GROUPED,
    MODE_PREFIX,
    PROMPT_VERSION,
    PROMPT_VERSION_GROUPED,
    FindingPromptInput,
    JudgeManifest,
    JudgeRecord,
    RuleMetadata,
    build_grouped_messages,
    build_grouped_prompt,
    build_messages,
    build_prompt,
    cache_key,
    candidate_sort_key,
    fewshots_file_sha,
    load_fewshots,
    load_judge_records,
    load_manifest,
    map_structured_verdict,
    messages_sha256,
    pack_sql,
    pack_sql_for_file,
    parse_grouped_verdicts,
    parse_structured_verdict,
    verdict_from_letter,
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
        "logprobs": {},
        "rule_description_sha": "ruledesc",
        "sql_sha": "sqlsha",
        "finding_span": "12:1-12",
        "runtime_version": "0.3.0",
        "mode": MODE_PREFIX,
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
                mode=record.mode,
                fewshots_sha=record.fewshots_sha,
            ),
        }
    )
    return record


def make_candidate(path: str, rule: str, line: int) -> CandidateFinding:
    return CandidateFinding(
        repo="spellbook",
        diagnostic={"path": path, "rule_id": rule, "line": line},
        bucket="other",
        registry_verdict=None,
    )


class JudgeLibTests(unittest.TestCase):
    def test_verdict_from_letter_c_abstains(self) -> None:
        verdict, reason = verdict_from_letter("C")
        self.assertEqual(verdict, "unsure")
        self.assertEqual(reason, "model_unsure")

    def test_verdict_from_letter_maps_tp_fp(self) -> None:
        verdict, reason = verdict_from_letter("A")
        self.assertEqual(verdict, "tp")
        self.assertIsNone(reason)
        verdict, reason = verdict_from_letter("B")
        self.assertEqual(verdict, "fp")
        self.assertIsNone(reason)

    def test_messages_sha256_stable(self) -> None:
        digest = messages_sha256("system", "user")
        self.assertEqual(len(digest), 64)
        self.assertEqual(digest, messages_sha256("system", "user"))

    def test_cache_key_differs_by_mode(self) -> None:
        shared = {
            "finding_id": "cgf_1",
            "rule_id": "SQLCOST012",
            "rule_description_sha": "abc",
            "sql_sha": "def",
            "finding_span": "1:1-1",
            "prompt_version": PROMPT_VERSION,
            "model_file_sha256": "model",
            "runtime_version": "0.3.0",
        }
        prefix = cache_key(**shared, mode=MODE_PREFIX)
        grouped = cache_key(**shared, mode=MODE_GROUPED)
        self.assertNotEqual(prefix, grouped)

    def test_cache_key_differs_by_fewshots_sha(self) -> None:
        shared = {
            "finding_id": "cgf_1",
            "rule_id": "SQLCOST012",
            "rule_description_sha": "abc",
            "sql_sha": "def",
            "finding_span": "1:1-1",
            "prompt_version": PROMPT_VERSION,
            "model_file_sha256": "model",
            "runtime_version": "0.3.0",
            "mode": MODE_PREFIX,
        }
        empty = cache_key(**shared, fewshots_sha="")
        hashed = cache_key(**shared, fewshots_sha="fewshots")
        self.assertNotEqual(empty, hashed)

    def test_map_structured_verdict_exemption_overrides(self) -> None:
        verdict, reason, token = map_structured_verdict(True, True, "A")
        self.assertEqual(verdict, "fp")
        self.assertIsNone(reason)
        self.assertEqual(token, "A")

    def test_map_structured_verdict_failure_condition_tp(self) -> None:
        verdict, reason, token = map_structured_verdict(False, True, "A")
        self.assertEqual(verdict, "tp")
        self.assertIsNone(reason)
        self.assertEqual(token, "A")

    def test_map_structured_verdict_c_abstains(self) -> None:
        verdict, reason, token = map_structured_verdict(False, False, "C")
        self.assertEqual(verdict, "unsure")
        self.assertEqual(reason, "model_unsure")
        self.assertEqual(token, "C")

    def test_parse_structured_verdict_malformed_defaults_c(self) -> None:
        parsed = parse_structured_verdict("not json")
        self.assertEqual(parsed.letter, "C")
        self.assertFalse(parsed.exemption_applies)

    def test_parse_structured_verdict_sqlcost012_exempt(self) -> None:
        raw = '{"exemption_applies": true, "failure_condition_met": false, "verdict": "B"}'
        parsed = parse_structured_verdict(raw)
        verdict, reason, _token = map_structured_verdict(
            parsed.exemption_applies,
            parsed.failure_condition_met,
            parsed.letter,
        )
        self.assertEqual(verdict, "fp")
        self.assertIsNone(reason)

    def test_load_fewshots_includes_rule_examples(self) -> None:
        text = load_fewshots("SQLCOST012", DEFAULT_FEWSHOTS)
        self.assertIn("UNNEST", text)
        self.assertIn("exemption_applies", text)

    def test_build_messages_includes_fewshots_and_json(self) -> None:
        meta = RuleMetadata(
            rule_id="SQLCOST012",
            title="Cross join",
            description="Detects cross joins",
            rubric="Intro\n\nAllow UNNEST patterns.",
        )
        _system, user = build_messages(
            meta,
            dialect="trino",
            line=10,
            span="10:1-10",
            message="cross join detected",
            sql="select 1",
        )
        sql_idx = user.index("SQL:")
        rule_idx = user.index("Rule ID:")
        self.assertLess(sql_idx, rule_idx)
        self.assertIn("Return JSON only", user)
        self.assertIn("Few-shot examples", user)
        self.assertIn("Current finding", user)

    def test_build_prompt_sql_before_rule(self) -> None:
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
        sql_idx = prompt.index("SQL:")
        rule_idx = prompt.index("Rule ID:")
        self.assertLess(sql_idx, rule_idx)
        self.assertNotIn("cross_join_unnest", prompt)

    def test_pack_sql_for_file_unions_windows(self) -> None:
        sql = "\n".join(f"line {idx}" for idx in range(1, 151))
        packed, digest, truncated, too_large = pack_sql_for_file(
            sql,
            [10, 11],
            sql_token_target=300,
        )
        self.assertTrue(truncated)
        self.assertFalse(too_large)
        self.assertIn("line 10", packed)
        self.assertIn("line 11", packed)
        self.assertLess(len(packed), len(sql))
        self.assertEqual(len(digest), 64)

    def test_pack_sql_for_file_too_large(self) -> None:
        sql = "\n".join(f"select {idx} from t" for idx in range(5000))
        packed, _digest, truncated, too_large = pack_sql_for_file(
            sql,
            list(range(1, 5000, 50)),
            sql_token_target=100,
        )
        self.assertTrue(truncated)
        self.assertTrue(too_large)
        self.assertLess(len(packed), len(sql))

    def test_parse_grouped_verdicts_defaults_missing_to_c(self) -> None:
        text = '[{"index":0,"verdict":"A"},{"index":2,"verdict":"B"}]'
        letters = parse_grouped_verdicts(text, 4)
        self.assertEqual(letters, ["A", "C", "B", "C"])

    def test_parse_grouped_verdicts_malformed_defaults_to_c(self) -> None:
        letters = parse_grouped_verdicts("not json at all", 2)
        self.assertEqual(letters, ["C", "C"])

    def test_candidate_sort_key_order(self) -> None:
        items = [
            make_candidate("b.sql", "SQLCOST002", 5),
            make_candidate("a.sql", "SQLCOST012", 10),
            make_candidate("a.sql", "SQLCOST005", 3),
        ]
        ordered = sorted(items, key=candidate_sort_key)
        self.assertEqual(ordered[0].diagnostic["path"], "a.sql")
        self.assertEqual(ordered[0].diagnostic["line"], 3)
        self.assertEqual(ordered[-1].diagnostic["path"], "b.sql")

    def test_cap_candidates_sorted(self) -> None:
        items = [
            make_candidate("z.sql", "SQLCOST012", 1),
            make_candidate("a.sql", "SQLCOST012", 1),
        ]
        capped = cap_candidates(items, cap=50, seed=3407)
        self.assertEqual(capped[0].diagnostic["path"], "a.sql")

    def test_build_grouped_messages_includes_indices(self) -> None:
        meta = RuleMetadata("SQLCOST012", "Cross join", "desc", "rubric")
        _system, user = build_grouped_messages(
            [FindingPromptInput(0, meta, 10, "10:1-10", "msg")],
            dialect="trino",
            sql="select 1",
        )
        self.assertIn("Finding index: 0", user)
        self.assertIn("JSON array", user)

    def test_build_grouped_prompt_includes_indices(self) -> None:
        meta = RuleMetadata("SQLCOST012", "Cross join", "desc", "rubric")
        prompt = build_grouped_prompt(
            [FindingPromptInput(0, meta, 10, "10:1-10", "msg")],
            dialect="trino",
            sql="select 1",
        )
        self.assertIn("Finding index: 0", prompt)

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
        self.assertEqual(loaded[0].mode, MODE_PREFIX)


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
                logprobs={},
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
        manifest = JudgeManifest(
            model_file_sha256="abc123",
            n_batch=2048,
            n_ubatch=512,
            sql_token_target=8000,
            mode=MODE_PREFIX,
            context_tokens=32768,
            flash_attn=True,
        )
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "manifest.toml"
            write_manifest(manifest, path)
            loaded = load_manifest(path)
        self.assertEqual(loaded.model_file_sha256, "abc123")
        self.assertEqual(loaded.n_batch, 2048)
        self.assertEqual(loaded.context_tokens, 32768)
        self.assertEqual(loaded.sql_token_target, 8000)
        self.assertTrue(loaded.flash_attn)
        self.assertEqual(loaded.mode, MODE_PREFIX)
        self.assertEqual(loaded.prompt_version, PROMPT_VERSION)

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_registry_fp_recall(self) -> None:
        manifest = JudgeManifest(model_file_sha256="modelsha", prompt_version=PROMPT_VERSION)
        records = [
            sample_record(registry_verdict="fp", llm_verdict="fp"),
            sample_record(
                finding_id="cgf_def",
                registry_verdict="fp",
                llm_verdict="tp",
                label_token="A",
            ),
            sample_record(
                finding_id="cgf_ghi",
                registry_verdict="tp",
                llm_verdict="tp",
                label_token="A",
            ),
        ]
        report = build_report(records, manifest)
        self.assertEqual(report["overall"]["registry_fp_recall"], 0.5)
        self.assertEqual(report["overall"]["registry_tp_recall"], 1.0)

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_grouped_prompt_version(self) -> None:
        manifest = JudgeManifest(mode=MODE_GROUPED, prompt_version=PROMPT_VERSION_GROUPED)
        self.assertEqual(manifest.prompt_version, PROMPT_VERSION_GROUPED)
        self.assertEqual(PROMPT_VERSION, "irr_judge_v3")

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install -r requirements-eval.txt)")
    def test_fewshots_sha_stable(self) -> None:
        digest = fewshots_file_sha(DEFAULT_FEWSHOTS)
        self.assertEqual(len(digest), 64)
        self.assertEqual(digest, fewshots_file_sha(DEFAULT_FEWSHOTS))


if __name__ == "__main__":
    unittest.main()

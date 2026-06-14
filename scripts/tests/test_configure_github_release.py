#!/usr/bin/env python3

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from configure_github_release import (  # noqa: E402
    EXPECTED_KEY_FINGERPRINT,
    RELEASE_OWNER,
    Configuration,
    RepositoryState,
    configuration_drift,
    desired_rulesets,
    load_allowed_signers,
    normalized_ruleset,
    ruleset_names,
)


class ConfigureGitHubReleaseTest(unittest.TestCase):
    def setUp(self) -> None:
        self.config = Configuration(
            organization="hypertrial",
            repository="costguard",
            profile="primary",
            allowed_signers=(ROOT / ".github/release_allowed_signers")
            .read_text(encoding="utf-8")
            .strip(),
        )
        self.team_id = 42
        self.owner_id = 76570855
        self.expected_rulesets = desired_rulesets(self.config, self.team_id)

    def compliant_state(self) -> RepositoryState:
        return RepositoryState(
            public=True,
            default_branch="main",
            owner_id=self.owner_id,
            team_id=self.team_id,
            team_members={RELEASE_OWNER},
            team_has_push=True,
            security={
                "advanced_security": True,
                "secret_scanning": True,
                "secret_scanning_push_protection": True,
                "private_vulnerability_reporting": True,
                "vulnerability_alerts": True,
                "automated_security_fixes": True,
            },
            release_environment={
                "protection_rules": [
                    {
                        "type": "required_reviewers",
                        "prevent_self_review": False,
                        "reviewers": [
                            {"type": "User", "reviewer": {"id": self.owner_id}}
                        ],
                    }
                ]
            },
            release_variable=self.config.allowed_signers,
            rulesets=self.expected_rulesets,
        )

    def test_existing_allowed_signer_matches_expected_fingerprint(self) -> None:
        value = load_allowed_signers(ROOT / ".github/release_allowed_signers")
        self.assertEqual(value, self.config.allowed_signers)
        self.assertEqual(
            EXPECTED_KEY_FINGERPRINT,
            "SHA256:uiM1q8pDCkb7iW+6sNTblHdSYh4h0XUocIFIsUu8gGc",
        )

    def test_allowed_signer_rejects_another_key(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "allowed_signers"
            path.write_text(
                "faltyn.matthew@gmail.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
                encoding="utf-8",
            )
            with self.assertRaises(SystemExit):
                load_allowed_signers(path)

    def test_compliant_state_has_no_drift(self) -> None:
        self.assertEqual(configuration_drift(self.compliant_state(), self.config), [])

    def test_extra_bypass_team_member_is_drift(self) -> None:
        state = self.compliant_state()
        state.team_members.add("mashkani")
        drift = configuration_drift(state, self.config)
        self.assertTrue(any("members" in item for item in drift))

    def test_release_environment_must_be_matt_owned_and_allow_self_review(self) -> None:
        state = self.compliant_state()
        state.release_environment = {
            "protection_rules": [
                {
                    "type": "required_reviewers",
                    "prevent_self_review": True,
                    "reviewers": [{"type": "User", "reviewer": {"id": 999}}],
                }
            ]
        }
        drift = configuration_drift(state, self.config)
        self.assertIn("configure release environment for Matt-only self approval", drift)

    def test_review_ruleset_has_team_bypass_and_required_checks(self) -> None:
        review_name, _ = ruleset_names("primary")
        review = self.expected_rulesets[review_name]
        self.assertEqual(
            review["bypass_actors"],
            [{"actor_id": self.team_id, "actor_type": "Team", "bypass_mode": "always"}],
        )
        checks = next(rule for rule in review["rules"] if rule["type"] == "required_status_checks")
        self.assertEqual(
            [item["context"] for item in checks["parameters"]["required_status_checks"]],
            ["pr-gate", "scale", "costguard"],
        )

    def test_integrity_ruleset_blocks_destructive_operations_without_bypass(self) -> None:
        _, integrity_name = ruleset_names("primary")
        integrity = self.expected_rulesets[integrity_name]
        self.assertEqual(integrity["bypass_actors"], [])
        self.assertEqual(
            {rule["type"] for rule in integrity["rules"]},
            {"deletion", "non_fast_forward"},
        )

    def test_ruleset_normalization_ignores_api_metadata(self) -> None:
        review_name, _ = ruleset_names("primary")
        expected = self.expected_rulesets[review_name]
        actual = {**expected, "id": 123, "_links": {"html": {"href": "example"}}}
        self.assertEqual(normalized_ruleset(actual), normalized_ruleset(expected))

    def test_private_repository_is_a_manual_visibility_blocker(self) -> None:
        state = self.compliant_state()
        state.public = False
        self.assertEqual(
            configuration_drift(state, self.config),
            ["hypertrial/costguard is private; visibility changes are intentionally manual"],
        )


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3

from __future__ import annotations

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class ActionContractTest(unittest.TestCase):
    def test_action_exposes_enterprise_install_and_compile_inputs(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        for expected in [
            "install-mode:",
            "version:",
            "dbt-requirements-file:",
            "dbt-constraints-file:",
            "dbt-vars:",
            "fail-on-deps-failure:",
            "use-existing-manifest:",
            "Install costguard release",
            "Build costguard CLI",
        ]:
            self.assertIn(expected, action)

    def test_repo_pr_workflow_uses_source_install_mode(self) -> None:
        workflow = (ROOT / ".github/workflows/costguard-pr.yml").read_text(encoding="utf-8")
        self.assertIn("install-mode: source", workflow)

    def test_release_action_and_workflow_share_asset_contract(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        release = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn('asset="costguard-${target}.tar.gz"', action)
        self.assertIn('asset="costguard-${{ matrix.target }}.tar.gz"', release)
        self.assertIn("releases/download/${version}", action)
        self.assertIn('.sha256"', action)


if __name__ == "__main__":
    unittest.main()

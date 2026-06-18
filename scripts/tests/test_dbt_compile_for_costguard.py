from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import dbt_compile_for_costguard as dbt_compile_module  # noqa: E402
from dbt_compile_for_costguard import (  # noqa: E402
    compile_dbt_for_costguard,
    compile_dbt_repo,
    manifest_cache_path,
    merge_manifests,
    packages_fingerprint,
    parse_compile_dirs,
    profile_type_from_adapter,
    read_dbt_profile_name,
    restore_manifest_cache,
    store_manifest_cache,
    write_dummy_profiles,
)


class DbtCompileHelpersTest(unittest.TestCase):
    def test_profile_type_from_adapter(self) -> None:
        self.assertEqual(profile_type_from_adapter("dbt-trino"), "trino")
        self.assertEqual(profile_type_from_adapter("dbt-postgres"), "postgres")
        self.assertEqual(profile_type_from_adapter("dbt-snowflake"), "snowflake")

    def test_parse_compile_dirs(self) -> None:
        self.assertEqual(parse_compile_dirs(""), [])
        self.assertEqual(parse_compile_dirs("alpha,beta"), ["alpha", "beta"])
        self.assertEqual(parse_compile_dirs("alpha\nbeta"), ["alpha", "beta"])

    def test_read_dbt_profile_name(self) -> None:
        project = self._temp_dir() / "proj"
        project.mkdir()
        (project / "dbt_project.yml").write_text(
            "name: demo\nprofile: my_profile\n",
            encoding="utf-8",
        )
        self.assertEqual(read_dbt_profile_name(project), "my_profile")

    def test_read_dbt_profile_name_allows_hyphens(self) -> None:
        project = self._temp_dir() / "proj"
        project.mkdir()
        (project / "dbt_project.yml").write_text(
            'name: demo\nprofile: "spellbook-local"\n',
            encoding="utf-8",
        )
        self.assertEqual(read_dbt_profile_name(project), "spellbook-local")

    def test_write_dummy_profiles(self) -> None:
        for profile_type in [
            "bigquery",
            "databricks",
            "duckdb",
            "postgres",
            "redshift",
            "snowflake",
            "trino",
        ]:
            profiles_dir = self._temp_dir() / profile_type
            write_dummy_profiles(
                profiles_dir,
                profile_name="demo",
                target="dev",
                profile_type=profile_type,
            )
            text = (profiles_dir / "profiles.yml").read_text(encoding="utf-8")
            self.assertIn(f"type: {profile_type}", text)

    def test_merge_manifests_prefixes_model_paths(self) -> None:
        tmp = self._temp_dir()
        alpha = tmp / "alpha.json"
        beta = tmp / "beta.json"
        alpha.write_text(
            json.dumps(
                {
                    "nodes": {
                        "model.alpha.one": {
                            "resource_type": "model",
                            "original_file_path": "models/one.sql",
                        }
                    },
                    "sources": {},
                    "exposures": {},
                }
            ),
            encoding="utf-8",
        )
        beta.write_text(
            json.dumps(
                {
                    "nodes": {
                        "model.beta.two": {
                            "resource_type": "model",
                            "original_file_path": "models/two.sql",
                        }
                    },
                    "sources": {},
                    "exposures": {},
                }
            ),
            encoding="utf-8",
        )
        output = tmp / "merged.json"
        merge_manifests([(alpha, "alpha"), (beta, "beta")], output)
        merged = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(
            merged["nodes"]["model.alpha.one"]["original_file_path"],
            "alpha/models/one.sql",
        )
        self.assertEqual(
            merged["nodes"]["model.beta.two"]["original_file_path"],
            "beta/models/two.sql",
        )

    def test_packages_fingerprint_stable(self) -> None:
        tmp = self._temp_dir()
        project = tmp / "dbt_subprojects" / "tokens"
        project.mkdir(parents=True)
        packages = project / "packages.yml"
        packages.write_text("packages:\n  - package: dbt-labs/dbt_utils\n", encoding="utf-8")
        first = packages_fingerprint(tmp, ["dbt_subprojects/tokens"], "dbt-trino")
        second = packages_fingerprint(tmp, ["dbt_subprojects/tokens"], "dbt-trino")
        self.assertEqual(first, second)
        self.assertNotEqual(
            packages_fingerprint(tmp, ["dbt_subprojects/tokens"], "dbt-trino", cache_scope="smoke"),
            packages_fingerprint(tmp, ["dbt_subprojects/tokens"], "dbt-trino", cache_scope="full"),
        )

    def test_manifest_cache_roundtrip(self) -> None:
        tmp = self._temp_dir()
        cache_dir = tmp / "cache"
        manifest_out = tmp / "checkout" / "target" / "manifest.json"
        manifest_out.parent.mkdir(parents=True)
        manifest_out.write_text('{"nodes": {}, "sources": {}, "exposures": {}}', encoding="utf-8")
        store_manifest_cache(
            cache_dir,
            "spellbook",
            "abc123",
            "deadbeef",
            manifest_out,
            compile_dirs=["dbt_subprojects/tokens"],
            adapter_package="dbt-trino",
        )
        restored = tmp / "checkout" / "target" / "manifest-restored.json"
        self.assertTrue(
            restore_manifest_cache(cache_dir, "spellbook", "abc123", "deadbeef", restored)
        )
        self.assertEqual(restored.read_text(encoding="utf-8"), manifest_out.read_text(encoding="utf-8"))
        self.assertTrue(
            (manifest_cache_path(cache_dir, "spellbook", "abc123", "deadbeef") / "meta.json").exists()
        )

    def test_use_existing_manifest_returns_without_compile(self) -> None:
        tmp = self._temp_dir()
        manifest = tmp / "target" / "manifest.json"
        manifest.parent.mkdir(parents=True)
        manifest.write_text('{"nodes": {}, "sources": {}, "exposures": {}}', encoding="utf-8")

        result, cache_state = compile_dbt_for_costguard(
            tmp,
            manifest_out=manifest,
            use_existing_manifest=True,
        )

        self.assertEqual(result, manifest)
        self.assertEqual(cache_state, "existing")

    def test_compile_project_passes_dbt_vars(self) -> None:
        tmp = self._temp_dir()
        project = tmp / "proj"
        project.mkdir()
        (project / "dbt_project.yml").write_text("name: demo\nprofile: demo\n", encoding="utf-8")
        dbt = tmp / "dbt"
        dbt.write_text(
            "#!/usr/bin/env python3\n"
            "import os\n"
            "import pathlib, sys\n"
            "pathlib.Path('dbt-args.txt').write_text(' '.join(sys.argv[1:]))\n"
            "pathlib.Path('dbt-env.txt').write_text(os.environ.get('COSTGUARD_DBT_FLAG', ''))\n"
            "if sys.argv[1] == 'compile':\n"
            "    pathlib.Path(sys.argv[sys.argv.index('--project-dir') + 1], 'target').mkdir(exist_ok=True)\n"
            "    pathlib.Path(sys.argv[sys.argv.index('--project-dir') + 1], 'target', 'manifest.json').write_text('{\"nodes\": {}}')\n",
            encoding="utf-8",
        )
        dbt.chmod(0o755)

        from dbt_compile_for_costguard import compile_dbt_project  # noqa: E402

        compile_dbt_project(
            tmp,
            project,
            dbt=dbt,
            target="dev",
            dbt_vars="{days: 7}",
            continue_on_deps_failure=False,
            dbt_env={"COSTGUARD_DBT_FLAG": "enabled"},
            no_introspect=False,
            no_populate_cache=False,
            threads=1,
        )

        args = (tmp / "dbt-args.txt").read_text(encoding="utf-8")
        self.assertIn("--vars {days: 7}", args)
        self.assertIn("--threads 1", args)
        self.assertNotIn("--no-introspect", args)
        self.assertNotIn("--no-populate-cache", args)
        self.assertEqual((tmp / "dbt-env.txt").read_text(encoding="utf-8"), "enabled")

    def test_compile_repo_passes_dbt_vars_and_scopes_cache(self) -> None:
        tmp = self._temp_dir()
        checkout = tmp / "checkout"
        checkout.mkdir()
        project = checkout / "integration_tests"
        project.mkdir()
        (project / "dbt_project.yml").write_text("name: demo\nprofile: default\n", encoding="utf-8")
        profiles = project / "profiles" / "duckdb"
        profiles.mkdir(parents=True)
        captured = {}

        original = dbt_compile_module.compile_dbt_for_costguard

        def fake_compile_dbt_for_costguard(_checkout: Path, **kwargs):
            captured.update(kwargs)
            return kwargs["manifest_out"], "miss"

        try:
            dbt_compile_module.compile_dbt_for_costguard = fake_compile_dbt_for_costguard
            state = compile_dbt_repo(
                checkout,
                {
                    "name": "tuva",
                    "commit": "a" * 40,
                    "compile_dbt": True,
                    "dbt_adapter": "dbt-duckdb",
                    "dbt_profile_type": "duckdb",
                    "dbt_project_dir": "integration_tests",
                    "dbt_profiles_dir": "integration_tests/profiles/duckdb",
                    "dbt_target": "ci",
                    "dbt_preserve_manifest_paths": True,
                    "dbt_env": {"DBT_MOTHERDUCK_CI_PATH": "costguard.duckdb"},
                    "dbt_no_introspect": False,
                    "dbt_no_populate_cache": False,
                    "dbt_threads": 1,
                    "dbt_vars": "{claims_enabled: true}",
                    "dbt_manifest_path": "docs/manifest.json",
                },
                cache_dir=tmp / "cache",
            )
        finally:
            dbt_compile_module.compile_dbt_for_costguard = original

        self.assertEqual(state, "miss")
        self.assertEqual(captured["dbt_vars"], "{claims_enabled: true}")
        self.assertIn("vars:{claims_enabled: true}", captured["cache_scope"])
        self.assertIn("project:integration_tests", captured["cache_scope"])
        self.assertIn("preserve-manifest-paths:true", captured["cache_scope"])
        self.assertIn("profiles:integration_tests/profiles/duckdb", captured["cache_scope"])
        self.assertIn("no-introspect:False", captured["cache_scope"])
        self.assertIn("no-populate-cache:False", captured["cache_scope"])
        self.assertIn("threads:1", captured["cache_scope"])
        self.assertIn("env:DBT_MOTHERDUCK_CI_PATH=costguard.duckdb", captured["cache_scope"])
        self.assertEqual(captured["project_dir"], project.resolve())
        self.assertEqual(captured["profiles_dir"], profiles.resolve())
        self.assertEqual(captured["target"], "ci")
        self.assertTrue(captured["preserve_manifest_paths"])
        self.assertFalse(captured["no_introspect"])
        self.assertFalse(captured["no_populate_cache"])
        self.assertEqual(captured["threads"], 1)
        self.assertEqual(captured["dbt_env"], {"DBT_MOTHERDUCK_CI_PATH": "costguard.duckdb"})
        self.assertEqual(captured["project_manifest"], (checkout / "docs" / "manifest.json").resolve())

    def test_compile_for_costguard_passes_project_manifest(self) -> None:
        tmp = self._temp_dir()
        checkout = tmp / "checkout"
        checkout.mkdir()
        manifest = checkout / "docs" / "manifest.json"
        manifest.parent.mkdir()
        manifest.write_text('{"nodes": {}, "sources": {}, "exposures": {}}', encoding="utf-8")
        output = checkout / "target" / "manifest.json"
        captured = {}

        original_dbt_tools = dbt_compile_module.dbt_tools
        original_compile_project = dbt_compile_module.compile_dbt_project

        def fake_dbt_tools(*_args, **_kwargs):
            return tmp, tmp / "dbt"

        def fake_compile_dbt_project(_checkout: Path, _project_dir: Path, **kwargs):
            captured.update(kwargs)
            return manifest

        try:
            dbt_compile_module.dbt_tools = fake_dbt_tools
            dbt_compile_module.compile_dbt_project = fake_compile_dbt_project
            result, state = compile_dbt_for_costguard(
                checkout,
                project_manifest=manifest,
                manifest_out=output,
                adapter_package="dbt-duckdb",
            )
        finally:
            dbt_compile_module.dbt_tools = original_dbt_tools
            dbt_compile_module.compile_dbt_project = original_compile_project

        self.assertEqual(result, output)
        self.assertEqual(state, "miss")
        self.assertEqual(captured["manifest_path"], manifest)

    def _temp_dir(self) -> Path:
        import tempfile

        return Path(tempfile.mkdtemp())


if __name__ == "__main__":
    unittest.main()

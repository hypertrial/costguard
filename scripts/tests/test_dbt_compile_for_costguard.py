from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from dbt_compile_for_costguard import (  # noqa: E402
    merge_manifests,
    parse_compile_dirs,
    profile_type_from_adapter,
    read_dbt_profile_name,
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

    def test_write_dummy_profiles(self) -> None:
        profiles_dir = self._temp_dir() / "profiles"
        write_dummy_profiles(
            profiles_dir,
            profile_name="demo",
            target="dev",
            profile_type="postgres",
        )
        text = (profiles_dir / "profiles.yml").read_text(encoding="utf-8")
        self.assertIn("type: postgres", text)

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

    def _temp_dir(self) -> Path:
        import tempfile

        return Path(tempfile.mkdtemp())


if __name__ == "__main__":
    unittest.main()

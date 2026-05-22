#!/usr/bin/env python3
"""Shared dbt compile and manifest merge helpers for Costguard CI and benchmarks."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


def read_dbt_profile_name(project_dir: Path) -> str:
    text = (project_dir / "dbt_project.yml").read_text(encoding="utf-8")
    match = re.search(r"^profile:\s*['\"]?(\w+)['\"]?", text, re.MULTILINE)
    return match.group(1) if match else "costguard"


def profile_type_from_adapter(adapter_package: str) -> str:
    normalized = adapter_package.removeprefix("dbt-").lower()
    aliases = {
        "bigquery": "bigquery",
        "postgres": "postgres",
        "redshift": "redshift",
        "snowflake": "snowflake",
        "trino": "trino",
        "duckdb": "duckdb",
        "databricks": "databricks",
    }
    return aliases.get(normalized, normalized)


def write_dummy_profiles(
    profiles_dir: Path,
    *,
    profile_name: str,
    target: str,
    profile_type: str,
) -> None:
    profiles_dir.mkdir(parents=True, exist_ok=True)
    profiles_file = profiles_dir / "profiles.yml"
    if profiles_file.exists():
        return
    profiles_file.write_text(
        f"""{profile_name}:
  target: {target}
  outputs:
    {target}:
      type: {profile_type}
      host: localhost
      port: 8080
      user: costguard
      database: costguard
      schema: costguard
""",
        encoding="utf-8",
    )


def venv_python() -> str:
    for candidate in ("python3.11", "python3.12", sys.executable):
        completed = subprocess.run(
            [
                candidate,
                "-c",
                "import sys; raise SystemExit(0 if sys.version_info < (3, 14) else 1)",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        if completed.returncode == 0:
            return candidate
    return sys.executable


def dbt_tools(cache_dir: Path, adapter: str) -> tuple[Path, Path]:
    venv_dir = cache_dir / ".dbt-venv"
    if not venv_dir.exists():
        created = subprocess.run(
            [venv_python(), "-m", "venv", str(venv_dir)],
            capture_output=True,
            text=True,
            check=False,
        )
        if created.returncode != 0:
            raise SystemExit(f"failed to create dbt venv:\n{created.stderr}")

    pip = venv_dir / "bin" / "pip"
    dbt = venv_dir / "bin" / "dbt"
    install = subprocess.run(
        [str(pip), "install", "--upgrade", "pip", adapter],
        capture_output=True,
        text=True,
        check=False,
    )
    if install.returncode != 0:
        raise SystemExit(f"failed to install {adapter}:\n{install.stderr}")
    return pip, dbt


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def merge_manifests(entries: list[tuple[Path, str]], output: Path) -> None:
    merged: dict[str, Any] = {"nodes": {}, "sources": {}, "exposures": {}}
    for manifest_path, prefix in entries:
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
        for section in ("nodes", "sources", "exposures"):
            for key, value in data.get(section, {}).items():
                if section == "nodes" and value.get("resource_type") == "model":
                    value = dict(value)
                    rel_path = value.get("original_file_path") or value.get("path")
                    if rel_path and prefix:
                        value["original_file_path"] = str(Path(prefix) / rel_path)
                merged[section][key] = value
    write_json(output, merged)


def compile_dbt_project(
    checkout: Path,
    project_dir: Path,
    *,
    dbt: Path,
    target: str = "dev",
    profile_type: str = "trino",
    profiles_dir: Path | None = None,
    profiles_rel: str = ".",
    continue_on_deps_failure: bool = True,
) -> Path:
    if not (project_dir / "dbt_project.yml").exists():
        raise SystemExit(f"compile enabled but no dbt_project.yml in {project_dir}")

    if (project_dir / "profiles.yml").exists():
        resolved_profiles_dir = project_dir
    elif profiles_dir is not None:
        resolved_profiles_dir = profiles_dir
    else:
        resolved_profiles_dir = (checkout / profiles_rel).resolve()
        profile_name = read_dbt_profile_name(project_dir)
        write_dummy_profiles(
            resolved_profiles_dir,
            profile_name=profile_name,
            target=target,
            profile_type=profile_type,
        )

    env = os.environ.copy()
    env["DBT_PROFILES_DIR"] = str(resolved_profiles_dir)

    deps = subprocess.run(
        [str(dbt), "deps", "--project-dir", str(project_dir)],
        cwd=checkout,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if deps.returncode != 0:
        message = f"warning: dbt deps failed for {project_dir}:\n{deps.stderr}"
        if continue_on_deps_failure:
            print(message, file=sys.stderr)
        else:
            raise SystemExit(message)

    compile_proc = subprocess.run(
        [str(dbt), "compile", "--project-dir", str(project_dir), "--target", target],
        cwd=checkout,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if compile_proc.returncode != 0:
        raise SystemExit(
            f"dbt compile failed for {project_dir} (exit {compile_proc.returncode}):\n"
            f"{compile_proc.stderr.strip()}"
        )

    manifest = project_dir / "target" / "manifest.json"
    if not manifest.exists():
        raise SystemExit(f"dbt compile succeeded but manifest missing at {manifest}")
    return manifest


def parse_compile_dirs(raw: str) -> list[str]:
    if not raw.strip():
        return []
    parts: list[str] = []
    for chunk in raw.replace("\n", ",").split(","):
        value = chunk.strip()
        if value:
            parts.append(value)
    return parts


def compile_dbt_for_costguard(
    checkout: Path,
    *,
    project_dir: Path | None = None,
    compile_dirs: list[str] | None = None,
    manifest_out: Path,
    adapter_package: str = "dbt-trino",
    profile_type: str | None = None,
    target: str = "dev",
    profiles_dir: Path | None = None,
    profiles_rel: str = ".",
    cache_dir: Path | None = None,
    continue_on_deps_failure: bool = True,
    use_system_dbt: bool = False,
) -> Path:
    resolved_profile_type = profile_type or profile_type_from_adapter(adapter_package)

    if use_system_dbt:
        dbt = Path("dbt")
    else:
        cache = cache_dir or (Path.home() / ".cache" / "costguard" / "dbt-venv")
        _, dbt = dbt_tools(cache, adapter_package)

    entries: list[tuple[Path, str]] = []
    dirs = compile_dirs or []
    if dirs:
        for rel in dirs:
            subproject = (checkout / rel).resolve()
            manifest = compile_dbt_project(
                checkout,
                subproject,
                dbt=dbt,
                target=target,
                profile_type=resolved_profile_type,
                profiles_dir=profiles_dir,
                profiles_rel=profiles_rel,
                continue_on_deps_failure=continue_on_deps_failure,
            )
            entries.append((manifest, rel))
        merge_manifests(entries, manifest_out)
        return manifest_out

    resolved_project = project_dir or checkout
    manifest = compile_dbt_project(
        checkout,
        resolved_project,
        dbt=dbt,
        target=target,
        profile_type=resolved_profile_type,
        profiles_dir=profiles_dir,
        profiles_rel=profiles_rel,
        continue_on_deps_failure=continue_on_deps_failure,
    )
    try:
        project_rel = str(resolved_project.relative_to(checkout))
    except ValueError:
        project_rel = ""
    if project_rel and project_rel != ".":
        merge_manifests([(manifest, project_rel)], manifest_out)
    else:
        write_json(manifest_out, json.loads(manifest.read_text(encoding="utf-8")))
    return manifest_out


def compile_dbt_repo(checkout: Path, repo: dict[str, Any], *, cache_dir: Path) -> None:
    if not repo.get("compile_dbt", False):
        return

    compile_dbt_for_costguard(
        checkout,
        compile_dirs=repo.get("dbt_compile_dirs"),
        project_dir=(checkout / repo.get("dbt_project_dir", ".")).resolve()
        if not repo.get("dbt_compile_dirs")
        else None,
        manifest_out=checkout / "target" / "manifest.json",
        adapter_package=repo.get("dbt_adapter", "dbt-trino"),
        profile_type=repo.get("dbt_profile_type"),
        target=repo.get("dbt_target", "dev"),
        profiles_rel=repo.get("dbt_profiles_dir", "."),
        cache_dir=cache_dir,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, default=Path.cwd())
    parser.add_argument("--project-dir", type=Path, default=None)
    parser.add_argument("--compile-dirs", default="")
    parser.add_argument("--adapter-package", default="dbt-trino")
    parser.add_argument("--profile-type", default="")
    parser.add_argument("--target", default="dev")
    parser.add_argument("--profiles-dir", type=Path, default=None)
    parser.add_argument("--profiles-rel", default=".")
    parser.add_argument("--manifest-out", type=Path, default=Path("target/manifest.json"))
    parser.add_argument("--cache-dir", type=Path, default=None)
    parser.add_argument("--use-system-dbt", action="store_true")
    parser.add_argument(
        "--fail-on-deps-failure",
        action="store_true",
        help="exit when dbt deps fails (default: warn and continue)",
    )
    args = parser.parse_args()

    checkout = args.checkout.resolve()
    manifest_out = args.manifest_out
    if not manifest_out.is_absolute():
        manifest_out = checkout / manifest_out

    project_dir = args.project_dir
    if project_dir is not None:
        project_dir = (checkout / project_dir).resolve() if not project_dir.is_absolute() else project_dir

    compile_dbt_for_costguard(
        checkout,
        project_dir=project_dir,
        compile_dirs=parse_compile_dirs(args.compile_dirs),
        manifest_out=manifest_out,
        adapter_package=args.adapter_package,
        profile_type=args.profile_type or None,
        target=args.target,
        profiles_dir=args.profiles_dir,
        profiles_rel=args.profiles_rel,
        cache_dir=args.cache_dir,
        continue_on_deps_failure=not args.fail_on_deps_failure,
        use_system_dbt=args.use_system_dbt,
    )
    print(f"Wrote manifest to {manifest_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

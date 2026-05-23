#!/usr/bin/env python3
"""Shared dbt compile and manifest merge helpers for Costguard CI and benchmarks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor, as_completed
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


def dbt_tools(
    cache_dir: Path,
    adapter: str,
    *,
    requirements_file: Path | None = None,
    constraints_file: Path | None = None,
) -> tuple[Path, Path]:
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
    install_fingerprint = {
        "adapter": adapter,
        "requirements_file": str(requirements_file) if requirements_file else "",
        "constraints_file": str(constraints_file) if constraints_file else "",
        "requirements_mtime": requirements_file.stat().st_mtime if requirements_file and requirements_file.exists() else "",
        "constraints_mtime": constraints_file.stat().st_mtime if constraints_file and constraints_file.exists() else "",
    }
    marker = venv_dir / f".installed-{hashlib.sha256(json.dumps(install_fingerprint, sort_keys=True).encode('utf-8')).hexdigest()[:16]}"
    if marker.exists():
        return pip, dbt
    upgrade = subprocess.run(
        [str(pip), "install", "--upgrade", "pip"],
        capture_output=True,
        text=True,
        check=False,
    )
    if upgrade.returncode != 0:
        raise SystemExit(f"failed to upgrade pip:\n{upgrade.stderr}")
    install_cmd = [str(pip), "install"]
    if constraints_file is not None:
        install_cmd.extend(["-c", str(constraints_file)])
    if requirements_file is not None:
        install_cmd.extend(["-r", str(requirements_file)])
    install_cmd.append(adapter)
    install = subprocess.run(
        install_cmd,
        capture_output=True,
        text=True,
        check=False,
    )
    if install.returncode != 0:
        raise SystemExit(f"failed to install {adapter}:\n{install.stderr}")
    marker.write_text(adapter, encoding="utf-8")
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
    dbt_vars: str = "",
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

    compile_cmd = [str(dbt), "compile", "--project-dir", str(project_dir), "--target", target]
    if dbt_vars.strip():
        compile_cmd.extend(["--vars", dbt_vars])
    compile_proc = subprocess.run(
        compile_cmd,
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


def packages_fingerprint(
    checkout: Path,
    compile_dirs: list[str],
    adapter_package: str,
    *,
    cache_scope: str = "",
) -> str:
    digest = hashlib.sha256()
    digest.update(adapter_package.encode("utf-8"))
    if cache_scope:
        digest.update(cache_scope.encode("utf-8"))
    for rel in sorted(compile_dirs):
        root = checkout / rel
        if not root.exists():
            continue
        for pattern in ("packages.yml", "package-lock.yml"):
            for path in sorted(root.rglob(pattern)):
                digest.update(str(path.relative_to(checkout)).encode("utf-8"))
                digest.update(path.read_bytes())
    return digest.hexdigest()[:16]


def manifest_cache_path(
    cache_dir: Path,
    repo_name: str,
    commit: str,
    packages_fp: str,
) -> Path:
    return cache_dir / "manifests" / repo_name / commit / packages_fp


def restore_manifest_cache(
    cache_dir: Path,
    repo_name: str,
    commit: str,
    packages_fp: str,
    manifest_out: Path,
) -> bool:
    cached = manifest_cache_path(cache_dir, repo_name, commit, packages_fp) / "manifest.json"
    if not cached.exists():
        return False
    manifest_out.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(cached, manifest_out)
    return True


def store_manifest_cache(
    cache_dir: Path,
    repo_name: str,
    commit: str,
    packages_fp: str,
    manifest_out: Path,
    *,
    compile_dirs: list[str],
    adapter_package: str,
) -> None:
    cache_root = manifest_cache_path(cache_dir, repo_name, commit, packages_fp)
    cache_root.mkdir(parents=True, exist_ok=True)
    shutil.copy2(manifest_out, cache_root / "manifest.json")
    write_json(
        cache_root / "meta.json",
        {
            "repo": repo_name,
            "commit": commit,
            "packages_fp": packages_fp,
            "compile_dirs": compile_dirs,
            "adapter_package": adapter_package,
        },
    )


def compile_jobs(count: int) -> int:
    forced = os.environ.get("COSTGUARD_DBT_COMPILE_JOBS", "").strip()
    if forced == "1":
        return 1
    if forced.isdigit():
        return max(1, min(int(forced), count))
    return max(1, min(count, os.cpu_count() or 4, 5))


def _compile_subproject_worker(args: tuple[str, str, str, str, str, str, str, bool, str]) -> tuple[str, str]:
    (
        checkout_s,
        rel,
        dbt_s,
        target,
        profile_type,
        profiles_dir_s,
        profiles_rel,
        continue_on_deps_failure,
        dbt_vars,
    ) = args
    checkout = Path(checkout_s)
    project_dir = (checkout / rel).resolve()
    profiles_dir = Path(profiles_dir_s) if profiles_dir_s else None
    manifest = compile_dbt_project(
        checkout,
        project_dir,
        dbt=Path(dbt_s),
        target=target,
        profile_type=profile_type,
        profiles_dir=profiles_dir,
        profiles_rel=profiles_rel,
        continue_on_deps_failure=continue_on_deps_failure,
        dbt_vars=dbt_vars,
    )
    return rel, str(manifest)


def compile_subprojects_parallel(
    checkout: Path,
    compile_dirs: list[str],
    *,
    dbt: Path,
    target: str,
    profile_type: str,
    profiles_dir: Path | None,
    profiles_rel: str,
    continue_on_deps_failure: bool,
    dbt_vars: str,
) -> list[tuple[Path, str]]:
    profiles_dir_s = str(profiles_dir) if profiles_dir is not None else ""
    worker_args = [
        (
            str(checkout),
            rel,
            str(dbt),
            target,
            profile_type,
            profiles_dir_s,
            profiles_rel,
            continue_on_deps_failure,
            dbt_vars,
        )
        for rel in compile_dirs
    ]
    jobs = compile_jobs(len(worker_args))
    entries: list[tuple[Path, str]] = []
    if jobs == 1 or len(worker_args) == 1:
        for args in worker_args:
            rel, manifest_s = _compile_subproject_worker(args)
            entries.append((Path(manifest_s), rel))
        return entries

    with ProcessPoolExecutor(max_workers=jobs) as pool:
        futures = [pool.submit(_compile_subproject_worker, args) for args in worker_args]
        for future in as_completed(futures):
            rel, manifest_s = future.result()
            entries.append((Path(manifest_s), rel))
    entries.sort(key=lambda item: item[1])
    return entries


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
    repo_name: str | None = None,
    commit: str | None = None,
    force_compile: bool = False,
    cache_scope: str = "",
    requirements_file: Path | None = None,
    constraints_file: Path | None = None,
    dbt_vars: str = "",
    use_existing_manifest: bool = False,
) -> tuple[Path, str]:
    resolved_profile_type = profile_type or profile_type_from_adapter(adapter_package)
    dirs = compile_dirs or []

    if use_existing_manifest:
        if manifest_out.exists():
            return manifest_out, "existing"
        raise SystemExit(f"use-existing-manifest requested but manifest missing at {manifest_out}")

    if (
        not force_compile
        and cache_dir is not None
        and repo_name
        and commit
        and dirs
    ):
        packages_fp = packages_fingerprint(
            checkout,
            dirs,
            adapter_package,
            cache_scope=cache_scope,
        )
        if restore_manifest_cache(cache_dir, repo_name, commit, packages_fp, manifest_out):
            return manifest_out, "hit"

    if use_system_dbt:
        dbt = Path("dbt")
    else:
        cache = cache_dir or (Path.home() / ".cache" / "costguard" / "dbt-venv")
        _, dbt = dbt_tools(
            cache,
            adapter_package,
            requirements_file=requirements_file,
            constraints_file=constraints_file,
        )

    entries: list[tuple[Path, str]] = []
    if dirs:
        entries = compile_subprojects_parallel(
            checkout,
            dirs,
            dbt=dbt,
            target=target,
            profile_type=resolved_profile_type,
            profiles_dir=profiles_dir,
            profiles_rel=profiles_rel,
            continue_on_deps_failure=continue_on_deps_failure,
            dbt_vars=dbt_vars,
        )
        merge_manifests(entries, manifest_out)
    else:
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
            dbt_vars=dbt_vars,
        )
        try:
            project_rel = str(resolved_project.relative_to(checkout))
        except ValueError:
            project_rel = ""
        if project_rel and project_rel != ".":
            merge_manifests([(manifest, project_rel)], manifest_out)
        else:
            write_json(manifest_out, json.loads(manifest.read_text(encoding="utf-8")))

    if cache_dir is not None and repo_name and commit and dirs:
        packages_fp = packages_fingerprint(
            checkout,
            dirs,
            adapter_package,
            cache_scope=cache_scope,
        )
        store_manifest_cache(
            cache_dir,
            repo_name,
            commit,
            packages_fp,
            manifest_out,
            compile_dirs=dirs,
            adapter_package=adapter_package,
        )
    return manifest_out, "miss"


def compile_dbt_repo(
    checkout: Path,
    repo: dict[str, Any],
    *,
    cache_dir: Path,
    smoke: bool = False,
    force_compile: bool = False,
) -> str:
    if not repo.get("compile_dbt", False):
        return "skip"

    compile_dirs = repo.get("dbt_compile_dirs")
    if smoke:
        compile_dirs = repo.get("smoke_compile_dirs") or compile_dirs

    _, compile_cache = compile_dbt_for_costguard(
        checkout,
        compile_dirs=compile_dirs,
        project_dir=(checkout / repo.get("dbt_project_dir", ".")).resolve()
        if not compile_dirs
        else None,
        manifest_out=checkout / "target" / "manifest.json",
        adapter_package=repo.get("dbt_adapter", "dbt-trino"),
        profile_type=repo.get("dbt_profile_type"),
        target=repo.get("dbt_target", "dev"),
        profiles_rel=repo.get("dbt_profiles_dir", "."),
        cache_dir=cache_dir,
        repo_name=repo["name"],
        commit=repo["commit"],
        force_compile=force_compile,
        cache_scope="smoke" if smoke else "",
    )
    return compile_cache


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
    parser.add_argument("--requirements-file", type=Path, default=None)
    parser.add_argument("--constraints-file", type=Path, default=None)
    parser.add_argument("--vars", default="")
    parser.add_argument("--use-existing-manifest", action="store_true")
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
        requirements_file=args.requirements_file,
        constraints_file=args.constraints_file,
        dbt_vars=args.vars,
        use_existing_manifest=args.use_existing_manifest,
    )
    print(f"Wrote manifest to {manifest_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

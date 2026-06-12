#!/usr/bin/env python3
"""Runtime driver for the Costguard composite GitHub Action."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

ADAPTERS = {
    "snowflake": "dbt-snowflake",
    "bigquery": "dbt-bigquery",
    "databricks": "dbt-databricks",
    "redshift": "dbt-redshift",
    "postgres": "dbt-postgres",
    "postgresql": "dbt-postgres",
    "duckdb": "dbt-duckdb",
    "trino": "dbt-trino",
    "presto": "dbt-trino",
}


def env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def action_path() -> Path:
    value = env("GITHUB_ACTION_PATH")
    if value:
        return Path(value).resolve()
    return Path(__file__).resolve().parents[1]


def action_repo_root() -> Path:
    return action_path().parents[2]


def ensure_scripts_path() -> None:
    scripts = action_repo_root() / "scripts"
    if str(scripts) not in sys.path:
        sys.path.insert(0, str(scripts))


def action_release_version() -> str:
    ensure_scripts_path()
    from costguard_tooling import release_tag_version  # noqa: E402

    return release_tag_version(action_repo_root())


def append_file(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(value + "\n")


def runner_target() -> tuple[str, str]:
    ensure_scripts_path()
    from costguard_tooling import host_target  # noqa: E402

    return host_target()


def download(url: str, destination: Path) -> None:
    with urllib.request.urlopen(url) as response, destination.open("wb") as output:
        shutil.copyfileobj(response, output)


def sha256(path: Path) -> str:
    ensure_scripts_path()
    from costguard_tooling import file_sha256  # noqa: E402

    return file_sha256(path)


def install_release(version: str) -> None:
    target, bin_name = runner_target()
    asset_name = f"costguard-{target}.tar.gz"
    base_url = env(
        "COSTGUARD_RELEASE_BASE_URL",
        f"https://github.com/hypertrial/costguard/releases/download/{version}",
    ).rstrip("/")
    runner_temp = Path(env("RUNNER_TEMP", tempfile.gettempdir()))
    install_dir = runner_temp / "costguard-bin"
    install_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="costguard-action-") as tmp:
        temp_dir = Path(tmp)
        asset = temp_dir / asset_name
        checksum = temp_dir / f"{asset_name}.sha256"
        download(f"{base_url}/{asset_name}", asset)
        download(f"{base_url}/{asset_name}.sha256", checksum)
        expected = checksum.read_text(encoding="utf-8").split()[0]
        actual = sha256(asset)
        if actual != expected:
            raise SystemExit(
                f"checksum mismatch for {asset_name}: expected {expected}, got {actual}"
            )
        with tarfile.open(asset, "r:gz") as archive:
            names = archive.getnames()
            if names != [bin_name]:
                raise SystemExit(f"unexpected archive layout: {names}")
            archive.extractall(install_dir, filter="data")
    if bin_name != "costguard.exe":
        (install_dir / bin_name).chmod(0o755)
    append_file(Path(env("GITHUB_PATH")), str(install_dir))


def install_source() -> None:
    root = action_repo_root()
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "-p", "costguard-cli"],
        cwd=root,
        check=True,
    )
    append_file(Path(env("GITHUB_PATH")), str(root / "target" / "release"))


def command_install(args: argparse.Namespace) -> int:
    if args.mode == "source":
        install_source()
        return 0
    if args.mode != "release":
        raise SystemExit(f"unknown install mode: {args.mode}")
    version = args.version or action_release_version()
    install_release(version)
    return 0


def resolve_adapter(warehouse: str, requested: str) -> str:
    if requested:
        return requested
    normalized = warehouse.lower()
    if normalized == "generic":
        raise SystemExit(
            "dbt-adapter-package is required when compile-dbt is enabled with warehouse=generic"
        )
    try:
        return ADAPTERS[normalized]
    except KeyError as exc:
        raise SystemExit(f"cannot derive dbt adapter for warehouse '{warehouse}'") from exc


def consumer_root() -> Path:
    workspace = Path(env("GITHUB_WORKSPACE", str(Path.cwd()))).resolve()
    working_directory = env("WORKING_DIRECTORY_INPUT", ".") or "."
    return (workspace / working_directory).resolve()


def compile_required() -> bool:
    if env("COMPILE_DBT_INPUT", "true").lower() != "true":
        return False
    if env("USE_EXISTING_MANIFEST_INPUT").lower() == "true":
        return False
    if env("MANIFEST_INPUT"):
        return False
    compile_dirs = env("DBT_COMPILE_DIRS_INPUT")
    project_dir = env("DBT_PROJECT_DIR_INPUT", ".") or "."
    return bool(compile_dirs) or (consumer_root() / project_dir / "dbt_project.yml").is_file()


def command_plan_compile() -> int:
    required = compile_required()
    adapter = ""
    if required:
        adapter = resolve_adapter(
            env("WAREHOUSE_INPUT", "generic"), env("DBT_ADAPTER_PACKAGE_INPUT")
        )
    output = Path(env("GITHUB_OUTPUT"))
    append_file(output, f"compile-required={'true' if required else 'false'}")
    append_file(output, f"adapter-package={adapter}")
    return 0


def command_compile() -> int:
    root = consumer_root()
    adapter = env("DBT_ADAPTER_PACKAGE_INPUT")
    if not adapter:
        raise SystemExit("resolved dbt adapter package is missing")

    install_args = [sys.executable, "-m", "pip", "install"]
    constraints = env("DBT_CONSTRAINTS_FILE_INPUT")
    requirements = env("DBT_REQUIREMENTS_FILE_INPUT")
    if constraints:
        install_args.extend(["-c", str(root / constraints)])
    if requirements:
        install_args.extend(["-r", str(root / requirements)])
    install_args.append(adapter)
    if env("COSTGUARD_ACTION_SKIP_DBT_INSTALL") != "1":
        subprocess.run([sys.executable, "-m", "pip", "install", "--upgrade", "pip"], check=True)
        subprocess.run(install_args, check=True)

    helper = action_repo_root() / "scripts" / "dbt_compile_for_costguard.py"
    command = [
        sys.executable,
        str(helper),
        "--checkout",
        str(root),
        "--adapter-package",
        adapter,
        "--target",
        env("DBT_TARGET_INPUT", "dev"),
        "--manifest-out",
        env("MANIFEST_OUTPUT_INPUT", "target/manifest.json"),
        "--use-system-dbt",
    ]
    profile_type = env("DBT_PROFILE_TYPE_INPUT")
    profiles_dir = env("DBT_PROFILES_DIR_INPUT")
    compile_dirs = env("DBT_COMPILE_DIRS_INPUT")
    project_dir = env("DBT_PROJECT_DIR_INPUT", ".") or "."
    dbt_vars = env("DBT_VARS_INPUT")
    if profile_type:
        command.extend(["--profile-type", profile_type])
    if profiles_dir:
        command.extend(["--profiles-dir", profiles_dir])
    if compile_dirs:
        command.extend(["--compile-dirs", compile_dirs])
    else:
        command.extend(["--project-dir", project_dir])
    if dbt_vars:
        command.extend(["--vars", dbt_vars])
    if env("FAIL_ON_DEPS_FAILURE_INPUT").lower() == "true":
        command.append("--fail-on-deps-failure")
    subprocess.run(command, check=True)
    return 0


def command_run() -> int:
    root = consumer_root()
    command = [
        "costguard",
        "pr",
        "--base",
        env("BASE_INPUT", "origin/main"),
        "--warehouse",
        env("WAREHOUSE_INPUT", "generic"),
        "--fail-on",
        env("FAIL_ON_INPUT", "high"),
        "--format",
        env("FORMAT_INPUT", "github"),
    ]
    min_confidence = env("MIN_CONFIDENCE_INPUT")
    if min_confidence:
        command.extend(["--min-confidence", min_confidence])
    baseline = env("BASELINE_INPUT")
    if baseline:
        command.extend(["--baseline", baseline])
    if env("COST_INPUT").lower() == "true":
        command.append("--cost")
    fail_on_cost_delta = env("FAIL_ON_COST_DELTA_INPUT")
    if fail_on_cost_delta:
        command.extend(["--fail-on-cost-delta", fail_on_cost_delta])
    manifest = env("MANIFEST_INPUT")
    manifest_output = env("MANIFEST_OUTPUT_INPUT", "target/manifest.json")
    if not manifest and (root / manifest_output).is_file():
        manifest = manifest_output
    if env("USE_EXISTING_MANIFEST_INPUT").lower() == "true" and not manifest:
        raise SystemExit(f"requested existing manifest does not exist: {manifest_output}")
    if manifest:
        if not (root / manifest).is_file():
            raise SystemExit(f"manifest does not exist: {manifest}")
        command.extend(["--manifest", manifest])
    completed = subprocess.run(command, cwd=root, capture_output=True, text=True, check=False)
    sys.stdout.write(completed.stdout)
    sys.stderr.write(completed.stderr)
    summary = env("GITHUB_STEP_SUMMARY")
    if env("FORMAT_INPUT") == "markdown" and summary:
        append_file(Path(summary), completed.stdout.rstrip("\n"))
    return completed.returncode


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    install = commands.add_parser("install")
    install.add_argument("--mode", required=True)
    install.add_argument("--version", default="")
    commands.add_parser("plan-compile")
    commands.add_parser("compile")
    commands.add_parser("run")
    return result


def main() -> int:
    args = parser().parse_args()
    if args.command == "install":
        return command_install(args)
    if args.command == "plan-compile":
        return command_plan_compile()
    if args.command == "compile":
        return command_compile()
    if args.command == "run":
        return command_run()
    raise AssertionError(args.command)


if __name__ == "__main__":
    raise SystemExit(main())

"""Shared release asset build, package, and verify helpers."""

from __future__ import annotations

import json
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

RELEASE_BIN_NAME = "costguard"
WINDOWS_BIN_NAME = "costguard.exe"

RELEASE_TARGETS: tuple[str, ...] = (
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
)

DOCS_RELEASE_TOOLCHAIN = "docs/book/reference/scripts.md"


def target_bin_name(target: str) -> str:
    if target.endswith("-pc-windows-msvc"):
        return WINDOWS_BIN_NAME
    return RELEASE_BIN_NAME


def asset_name(target: str) -> str:
    return f"costguard-{target}.tar.gz"


def host_target() -> tuple[str, str]:
    system = platform.system()
    machine = platform.machine().lower()
    if system == "Linux" and machine in {"x86_64", "amd64"}:
        return "x86_64-unknown-linux-gnu", RELEASE_BIN_NAME
    if system == "Darwin" and machine == "arm64":
        return "aarch64-apple-darwin", RELEASE_BIN_NAME
    if system == "Darwin" and machine == "x86_64":
        return "x86_64-apple-darwin", RELEASE_BIN_NAME
    if system in {"Windows", "MINGW", "MSYS", "CYGWIN"} and machine in {"x86_64", "amd64"}:
        return "x86_64-pc-windows-msvc", WINDOWS_BIN_NAME
    raise SystemExit(f"unsupported host platform for release verification: {system}-{machine}")


def installed_rust_targets() -> set[str]:
    proc = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise SystemExit(f"failed to list rust targets:\n{proc.stderr}")
    return {line.strip() for line in proc.stdout.splitlines() if line.strip()}


def ensure_targets_installed(targets: tuple[str, ...] = RELEASE_TARGETS) -> None:
    installed = installed_rust_targets()
    missing = [target for target in targets if target not in installed]
    if missing:
        joined = ", ".join(missing)
        raise SystemExit(
            "missing Rust targets for strict release build: "
            f"{joined}. Install with rustup target add and see {DOCS_RELEASE_TOOLCHAIN}."
        )


def command_exists(name: str) -> bool:
    return shutil.which(name) is not None


def cargo_target_dir(root: Path) -> Path:
    override = os.environ.get("CARGO_TARGET_DIR")
    if override:
        return Path(override)
    return root / "target"


def built_binary_path(root: Path, target: str) -> Path:
    return cargo_target_dir(root) / target / "release" / target_bin_name(target)


def build_target(root: Path, target: str) -> Path:
    host, _ = host_target()
    if target == host:
        cmd = [
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "costguard-cli",
            "--target",
            target,
        ]
    elif target == "x86_64-unknown-linux-gnu":
        if not command_exists("cargo-zigbuild"):
            raise SystemExit(
                "cargo-zigbuild is required to cross-compile "
                f"{target}. See {DOCS_RELEASE_TOOLCHAIN}."
            )
        if not command_exists("zig"):
            raise SystemExit(
                "zig is required to cross-compile "
                f"{target}. See {DOCS_RELEASE_TOOLCHAIN}."
            )
        cmd = [
            "cargo",
            "zigbuild",
            "--release",
            "--locked",
            "-p",
            "costguard-cli",
            "--target",
            target,
        ]
    elif target == "x86_64-pc-windows-msvc":
        if not command_exists("cargo-xwin"):
            raise SystemExit(
                "cargo-xwin is required to cross-compile "
                f"{target}. See {DOCS_RELEASE_TOOLCHAIN}."
            )
        cmd = [
            "cargo",
            "xwin",
            "build",
            "--release",
            "--locked",
            "-p",
            "costguard-cli",
            "--target",
            target,
        ]
    else:
        cmd = [
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "costguard-cli",
            "--target",
            target,
        ]

    proc = subprocess.run(cmd, cwd=root, check=False)
    if proc.returncode != 0:
        raise SystemExit(f"cargo build failed for target {target}")

    binary = built_binary_path(root, target)
    if not binary.exists():
        raise SystemExit(f"expected release binary at {binary}")
    return binary


def package_built_binary(
    workdir: Path,
    *,
    target: str,
    binary_path: Path,
) -> tuple[Path, Path]:
    bin_name = target_bin_name(target)
    dist_dir = workdir / target
    dist_dir.mkdir(parents=True, exist_ok=True)
    packaged_bin = dist_dir / bin_name
    shutil.copy2(binary_path, packaged_bin)
    if bin_name != WINDOWS_BIN_NAME:
        packaged_bin.chmod(0o755)

    asset = workdir / asset_name(target)
    with tarfile.open(asset, "w:gz") as archive:
        archive.add(packaged_bin, arcname=bin_name)

    checksum_file = workdir / f"{asset.name}.sha256"
    digest = subprocess.run(
        ["shasum", "-a", "256", asset.name],
        cwd=workdir,
        capture_output=True,
        text=True,
        check=False,
    )
    if digest.returncode != 0:
        raise SystemExit(f"failed to compute sha256:\n{digest.stderr}")
    checksum_file.write_text(digest.stdout, encoding="utf-8")
    return asset, checksum_file


def verify_checksum(workdir: Path, asset: Path, checksum_file: Path) -> None:
    verify = subprocess.run(
        ["shasum", "-a", "256", "-c", checksum_file.name],
        cwd=workdir,
        capture_output=True,
        text=True,
        check=False,
    )
    if verify.returncode != 0:
        raise SystemExit(
            "checksum verification failed:\n"
            f"{verify.stdout}\n{verify.stderr}"
        )


def verify_archive_layout(asset: Path, *, bin_name: str) -> None:
    with tarfile.open(asset, "r:gz") as archive:
        names = archive.getnames()
    if names != [bin_name]:
        raise SystemExit(f"expected archive root [{bin_name}], got {names}")


def can_smoke_test_target(target: str) -> bool:
    system = platform.system()
    if target == "x86_64-pc-windows-msvc":
        return system == "Windows"
    if target == "x86_64-unknown-linux-gnu":
        return system == "Linux"
    if target.endswith("-apple-darwin"):
        return system == "Darwin"
    return False


def extract_and_smoke_test(asset: Path, *, bin_name: str, target: str) -> None:
    if not can_smoke_test_target(target):
        verify_archive_layout(asset, bin_name=bin_name)
        return

    with tempfile.TemporaryDirectory(prefix="costguard-release-") as tmp:
        extract_dir = Path(tmp)
        with tarfile.open(asset, "r:gz") as archive:
            archive.extractall(extract_dir)
        binary = extract_dir / bin_name
        if not binary.exists():
            raise SystemExit(f"expected binary {bin_name} at archive root")
        proc = subprocess.run(
            [str(binary), "rules", "--format", "json"],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            raise SystemExit(
                "extracted binary failed rules smoke test:\n"
                f"{proc.stderr or proc.stdout}"
            )
        payload = json.loads(proc.stdout)
        if not isinstance(payload, list) or not payload:
            raise SystemExit("rules output was empty or invalid")


def package_and_verify_target(
    root: Path,
    workdir: Path,
    target: str,
    *,
    build: bool = True,
) -> tuple[Path, Path]:
    binary_path = build_target(root, target) if build else built_binary_path(root, target)
    if not binary_path.exists():
        raise SystemExit(f"release binary missing at {binary_path}")
    asset, checksum = package_built_binary(workdir, target=target, binary_path=binary_path)
    verify_checksum(workdir, asset, checksum)
    extract_and_smoke_test(asset, bin_name=target_bin_name(target), target=target)
    return asset, checksum

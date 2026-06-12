"""Shared deterministic release asset build, package, and verification helpers."""

from __future__ import annotations

import gzip
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]

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
    return WINDOWS_BIN_NAME if target.endswith("-pc-windows-msvc") else RELEASE_BIN_NAME


def asset_name(target: str) -> str:
    return f"costguard-{target}.tar.gz"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


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
    missing = [target for target in targets if target not in installed_rust_targets()]
    if missing:
        raise SystemExit(
            "missing Rust targets for strict release build: "
            f"{', '.join(missing)}. See {DOCS_RELEASE_TOOLCHAIN}."
        )


def command_exists(name: str) -> bool:
    return shutil.which(name) is not None


def cargo_target_dir(root: Path) -> Path:
    return Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))


def built_binary_path(root: Path, target: str) -> Path:
    return cargo_target_dir(root) / target / "release" / target_bin_name(target)


def build_command(target: str) -> list[str]:
    host, _ = host_target()
    if target == host or target.endswith("-apple-darwin"):
        prefix = ["cargo", "build"]
    elif target == "x86_64-unknown-linux-gnu":
        if not command_exists("cargo-zigbuild") or not command_exists("zig"):
            raise SystemExit(f"cargo-zigbuild and zig are required for {target}")
        prefix = ["cargo", "zigbuild"]
    elif target == "x86_64-pc-windows-msvc":
        if not command_exists("cargo-xwin"):
            raise SystemExit(f"cargo-xwin is required for {target}")
        prefix = ["cargo", "xwin", "build"]
    else:
        prefix = ["cargo", "build"]
    return [*prefix, "--release", "--locked", "-p", "costguard-cli", "--target", target]


def build_target(root: Path, target: str) -> Path:
    command = build_command(target)
    proc = subprocess.run(command, cwd=root, check=False)
    if proc.returncode != 0:
        raise SystemExit(f"release build failed for target {target}")
    binary = built_binary_path(root, target)
    if not binary.exists():
        raise SystemExit(f"expected release binary at {binary}")
    return binary


def source_date_epoch(default: int = 0) -> int:
    raw = os.environ.get("SOURCE_DATE_EPOCH")
    return int(raw) if raw else default


def package_built_binary(
    workdir: Path,
    *,
    target: str,
    binary_path: Path,
    epoch: int | None = None,
) -> tuple[Path, Path]:
    workdir.mkdir(parents=True, exist_ok=True)
    bin_name = target_bin_name(target)
    timestamp = source_date_epoch() if epoch is None else epoch
    asset = workdir / asset_name(target)
    info = tarfile.TarInfo(bin_name)
    info.size = binary_path.stat().st_size
    info.mode = 0o755
    info.uid = 0
    info.gid = 0
    info.uname = "root"
    info.gname = "root"
    info.mtime = timestamp
    with asset.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=timestamp) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                with binary_path.open("rb") as binary:
                    archive.addfile(info, binary)
    checksum_file = workdir / f"{asset.name}.sha256"
    checksum_file.write_text(f"{file_sha256(asset)}  {asset.name}\n", encoding="utf-8")
    return asset, checksum_file


def verify_checksum(workdir: Path, asset: Path, checksum_file: Path) -> None:
    expected = checksum_file.read_text(encoding="utf-8").split()[0]
    actual = file_sha256(asset)
    if actual != expected:
        raise SystemExit(f"checksum verification failed for {asset.name}")


def verify_archive_layout(asset: Path, *, bin_name: str) -> None:
    with tarfile.open(asset, "r:gz") as archive:
        names = archive.getnames()
        if names != [bin_name]:
            raise SystemExit(f"expected archive root [{bin_name}], got {names}")
        member = archive.getmember(bin_name)
        if member.uid != 0 or member.gid != 0 or member.mode & 0o777 != 0o755:
            raise SystemExit(f"non-deterministic archive metadata for {asset.name}")


def can_smoke_test_target(target: str) -> bool:
    system = platform.system()
    if target == "x86_64-pc-windows-msvc":
        return system == "Windows"
    if target == "x86_64-unknown-linux-gnu":
        return system == "Linux"
    return target.endswith("-apple-darwin") and system == "Darwin"


def extract_and_smoke_test(asset: Path, *, bin_name: str, target: str) -> None:
    verify_archive_layout(asset, bin_name=bin_name)
    if not can_smoke_test_target(target):
        return
    with tempfile.TemporaryDirectory(prefix="costguard-release-") as tmp:
        extract_dir = Path(tmp)
        with tarfile.open(asset, "r:gz") as archive:
            archive.extractall(extract_dir, filter="data")
        binary = extract_dir / bin_name
        if platform.system() == "Darwin" and target == "x86_64-apple-darwin":
            command = ["arch", "-x86_64", str(binary)]
        else:
            command = [str(binary)]
        proc = subprocess.run(
            [*command, "rules", "--format", "json"],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0 or not json.loads(proc.stdout):
            raise SystemExit(f"extracted binary smoke test failed:\n{proc.stderr or proc.stdout}")


def package_and_verify_target(
    root: Path,
    workdir: Path,
    target: str,
    *,
    build: bool = True,
    epoch: int | None = None,
) -> tuple[Path, Path]:
    binary = build_target(root, target) if build else built_binary_path(root, target)
    if not binary.exists():
        raise SystemExit(f"release binary missing at {binary}")
    asset, checksum = package_built_binary(
        workdir, target=target, binary_path=binary, epoch=epoch
    )
    verify_checksum(workdir, asset, checksum)
    verify_archive_layout(asset, bin_name=target_bin_name(target))
    return asset, checksum


def write_consolidated_checksums(workdir: Path, assets: list[Path]) -> Path:
    output = workdir / "SHA256SUMS"
    output.write_text(
        "".join(f"{file_sha256(path)}  {path.name}\n" for path in sorted(assets)),
        encoding="utf-8",
    )
    return output


def workspace_version(root: Path) -> str:
    data = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    return data["workspace"]["package"]["version"]


def command_output(command: list[str], *, cwd: Path) -> str:
    return subprocess.run(
        command, cwd=cwd, capture_output=True, text=True, check=True
    ).stdout.strip()


def optional_command_output(command: list[str], *, cwd: Path) -> str | None:
    if not command_exists(command[0]):
        return None
    completed = subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        return None
    return (completed.stdout or completed.stderr).strip()


def write_provenance(
    root: Path,
    workdir: Path,
    *,
    version: str,
    assets: list[Path],
    receipts: list[Path],
) -> Path:
    payload = {
        "schema_version": 1,
        "version": version,
        "commit": command_output(["git", "rev-parse", "HEAD"], cwd=root),
        "source_date_epoch": int(command_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=root)),
        "toolchains": {
            "rustc": command_output(["rustc", "--version"], cwd=root),
            "cargo": command_output(["cargo", "--version"], cwd=root),
            "rustup": command_output(["rustup", "--version"], cwd=root).splitlines()[0],
            "python": platform.python_version(),
            "zig": optional_command_output(["zig", "version"], cwd=root),
            "cargo-zigbuild": optional_command_output(
                ["cargo", "zigbuild", "--version"], cwd=root
            ),
            "cargo-xwin": optional_command_output(
                ["cargo", "xwin", "--version"], cwd=root
            ),
        },
        "targets": list(RELEASE_TARGETS),
        "build_commands": {target: build_command(target) for target in RELEASE_TARGETS},
        "assets": {path.name: file_sha256(path) for path in sorted(assets)},
        "receipts": {path.name: file_sha256(path) for path in sorted(receipts)},
        "verification_results": {
            path.name: json.loads(path.read_text(encoding="utf-8"))
            for path in sorted(receipts)
        },
    }
    output = workdir / "provenance.json"
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return output

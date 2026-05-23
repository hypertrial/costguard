#!/usr/bin/env python3
"""Build strict four-target release assets locally and optionally publish with gh."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from release_packaging import (  # noqa: E402
    RELEASE_TARGETS,
    asset_name,
    ensure_targets_installed,
    package_and_verify_target,
)


def require_command(name: str) -> None:
    from shutil import which

    if which(name) is None:
        raise SystemExit(f"required command not found: {name}")


def package_all_targets(root: Path, workdir: Path) -> list[Path]:
    workdir.mkdir(parents=True, exist_ok=True)
    assets: list[Path] = []
    for target in RELEASE_TARGETS:
        asset, checksum = package_and_verify_target(root, workdir, target)
        assets.extend([asset, checksum])
        print(f"packaged {asset.name}")
    return assets


def release_exists(version: str) -> bool:
    proc = subprocess.run(
        ["gh", "release", "view", version],
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.returncode == 0


def publish_release(
    version: str,
    workdir: Path,
    *,
    notes_file: Path | None,
) -> None:
    require_command("gh")
    assets = sorted(workdir.glob("costguard-*.tar.gz")) + sorted(
        workdir.glob("costguard-*.tar.gz.sha256")
    )
    if len(assets) != len(RELEASE_TARGETS) * 2:
        raise SystemExit(
            f"expected {len(RELEASE_TARGETS) * 2} release assets in {workdir}, found {len(assets)}"
        )

    if not release_exists(version):
        create_cmd = [
            "gh",
            "release",
            "create",
            version,
            "--title",
            f"Costguard {version}",
        ]
        if notes_file is not None:
            create_cmd.extend(["--notes-file", str(notes_file)])
        else:
            create_cmd.extend(["--notes", f"Costguard {version} release binaries."])
        proc = subprocess.run(create_cmd, check=False)
        if proc.returncode != 0:
            raise SystemExit(f"gh release create failed for {version}")

    upload_cmd = ["gh", "release", "upload", version, *[str(path) for path in assets], "--clobber"]
    proc = subprocess.run(upload_cmd, check=False)
    if proc.returncode != 0:
        raise SystemExit(f"gh release upload failed for {version}")
    print(f"published {len(assets)} assets to {version}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default="v0.1.0")
    parser.add_argument("--workdir", type=Path, default=Path("dist/release"))
    parser.add_argument(
        "--package-only",
        action="store_true",
        help="Build and verify assets without calling gh",
    )
    parser.add_argument(
        "--publish",
        action="store_true",
        help="Upload assets with gh after all targets pass",
    )
    parser.add_argument("--notes-file", type=Path, default=None)
    args = parser.parse_args()

    if args.publish and args.package_only:
        raise SystemExit("choose either --package-only or --publish")

    require_command("cargo")
    require_command("rustup")
    require_command("shasum")
    ensure_targets_installed(RELEASE_TARGETS)

    package_all_targets(ROOT, args.workdir)

    if args.publish:
        publish_release(args.version, args.workdir, notes_file=args.notes_file)
    elif not args.package_only:
        parser.error("specify --package-only or --publish")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

# Release checklist

GitHub-hosted Actions are optional manual mirrors. Local scripts are authoritative.

1. Merge the release-hardening change.
2. Make the repository public and enable private vulnerability reporting in GitHub repository settings.
3. Configure Git tag and provenance signing through `user.signingkey` and `gpg.format`.
4. Create and verify a signed annotated `v1.1.0` tag at the merge commit.
5. Run `python3 scripts/release_check.py --version 1.1.0` to create `dist/release/release-check.json`.
6. Run `./scripts/publish_release_local.sh --package-only --version 1.1.0` to build deterministic assets and local smoke receipts.
7. Transfer the MSVC archive to Windows, run `scripts/smoke_release_asset.py`, and return its receipt unchanged.
8. Run `./scripts/publish_release_local.sh --publish --version 1.1.0 --receipt PATH_TO_WINDOWS_RECEIPT`.
9. Run the published Action and binary download from a separate public consumer repository.
10. Create or move the signed `v1` tag to the same commit only after the live smoke test passes.

Strict qualification requires a clean worktree, the exact signed tag at `HEAD`, `mdbook`, and `cargo-deny`. Development-only skip flags cannot create release evidence. The publisher runs native macOS ARM, macOS x86 through Rosetta, and Linux x86 through Docker smoke tests. Final publication creates sidecar checksums, `SHA256SUMS`, four smoke receipts, a signed provenance manifest, and an exact immutable asset inventory.

# Release checklist

GitHub Actions is the sole publication authority. Local scripts qualify code and recover deterministic packages but cannot publish.

1. Merge the release candidate commit to `main`; require `pr-gate` and `costguard-pr` branch checks.
2. Complete full-history secret/customer-data scanning, then make the repository public and enable private vulnerability reporting, dependency alerts, push protection, and branch rules.
3. Produce three consecutive successful `ci.yml` runs for the exact release commit. The release workflow enforces this by commit SHA.
4. Configure `RELEASE_SSH_ALLOWED_SIGNERS`, create a signed annotated `v2.0.0-rc.1` tag, and push it.
5. Confirm the workflow publishes the exact tag as a GitHub prerelease and passes Linux, macOS ARM/x86, and Windows consumer smoke with attestations. RC tags never move `v2`.
6. Exercise startup standard mode and enterprise strict signed-policy mode in the public consumer repository.
7. Soak the RC for at least seven days with no unresolved critical/high correctness or security issues. Any runtime change requires `rc.2` and restarts the soak.
8. GA may differ from the final RC only by version, changelog, and release documentation. Tag immutable `v2.0.0`; after publication and the consumer matrix pass, the workflow moves `v2`.
9. Never replace an exact release. Publish post-GA fixes as `2.0.1` and move `v2` after verification.

Strict qualification requires a clean worktree, the exact signed tag at `HEAD`, `mdbook`, and `cargo-deny`. Development skip flags cannot create release evidence. Local recovery packaging uses `./scripts/publish_release_local.sh --package-only`; publication remains disabled outside GitHub Actions.

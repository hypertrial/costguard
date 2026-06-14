# Release checklist

GitHub Actions is the sole publication authority. Local scripts qualify code and recover deterministic packages but cannot publish.

Matt (`mattfaltyn`) is the sole release owner. The `release-owners` team contains only Matt and bypasses PR, review, and required-check rules, so Matt may self-merge or push directly to `main`. Other contributors require one approval plus successful `pr-gate`, `scale`, and `costguard` checks. Force pushes and default-branch deletion have no bypass. A direct push receives CI feedback after entering `main`; fix failures with a new commit, never by rewriting history.

1. Merge or directly push the release candidate commit to `main` as Matt. PRs are recommended for reviewability but do not require another approver.
2. Complete full-history secret/customer-data scanning, then explicitly make the repository public. Run `configure_github_release.py` in `--plan`, `--apply`, and `--verify` modes to enable public security features, the Matt-only bypass, branch rules, the release environment, and `RELEASE_SSH_ALLOWED_SIGNERS`.
3. Produce exactly one successful push run and two successful workflow-dispatch `ci.yml` runs for the release commit. Every run must complete `pr-gate`, `scale`, and `spellbook-smoke`; the release workflow enforces this by commit SHA.
4. Use Matt's existing passphrase-protected `~/.ssh/id_ed25519` key to create signed annotated `v2.0.0-rc.2`. Do not add another key or change global Git configuration.
5. Confirm the workflow publishes the exact tag as a GitHub prerelease and passes Linux, macOS ARM/x86, and Windows consumer smoke with attestations. RC tags never move `v2`.
6. Exercise startup standard mode and enterprise strict signed-policy mode in the public consumer repository.
7. Record the successful publication timestamp and soak the RC for at least seven full days with no unresolved critical/high correctness or security issues. Any runtime change requires the next RC and restarts the soak.
8. GA may differ from the final RC only by version, changelog, and release documentation. Tag immutable `v2.0.0`; after publication and the consumer matrix pass, the workflow moves `v2`.
9. Never replace an exact release. Publish post-GA fixes as `2.0.1` and move `v2` after verification.

Strict qualification requires a clean worktree, the exact signed tag at `HEAD`, `mdbook`, and `cargo-deny`. Development skip flags cannot create release evidence. Local recovery packaging uses `./scripts/publish_release_local.sh --package-only`; publication remains disabled outside GitHub Actions.

## Public repository controls

The configuration command requires a GitHub token with `repo`, `admin:org`, and repository administration access. It intentionally refuses private repositories and never changes visibility.

```bash
export GH_TOKEN="$(gh auth token)"
python3 scripts/configure_github_release.py --plan
python3 scripts/configure_github_release.py --apply
python3 scripts/configure_github_release.py --verify
```

Apply the consumer profile after creating `hypertrial/costguard-consumer-smoke`:

```bash
python3 scripts/configure_github_release.py \
  --repository hypertrial/costguard-consumer-smoke \
  --profile consumer \
  --apply
```

The tracked [allowed-signers file](../../../.github/release_allowed_signers) contains only Matt's existing public key. Its expected fingerprint is `SHA256:uiM1q8pDCkb7iW+6sNTblHdSYh4h0XUocIFIsUu8gGc`. The setup script rejects another key, extra release-team members, disabled security controls, or ruleset drift.

## RC2 signing and qualification

Dispatch `ci.yml` twice only after the main push run succeeds, and wait for each dispatch before starting the next. The tag command prompts to unlock the existing key when necessary:

```bash
git -c gpg.format=ssh \
  -c user.signingkey="$HOME/.ssh/id_ed25519" \
  tag -s -m "Costguard v2.0.0-rc.2" v2.0.0-rc.2

git -c gpg.format=ssh \
  -c gpg.ssh.allowedSignersFile=.github/release_allowed_signers \
  verify-tag v2.0.0-rc.2

git push origin refs/tags/v2.0.0-rc.2
```

After publication and both public consumer jobs pass, record the GitHub release `published_at` timestamp as the soak start. GA is prohibited until seven complete 24-hour periods have elapsed.

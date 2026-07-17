# Release checklist

GitHub Actions is the sole publication authority. Local scripts qualify code and recover deterministic packages but cannot publish.

Matt (`mattfaltyn`) is the sole release owner. The `release-owners` team contains only Matt and bypasses PR, review, and required-check rules, so Matt may self-merge or push directly to `main`. Other contributors require one approval plus successful `pr-gate` and `costguard` checks; synthetic scale is part of `pr-gate`. Force pushes and default-branch deletion have no bypass. A direct push receives CI feedback after entering `main`; fix failures with a new commit, never by rewriting history.

The current public docs pin `v2.6.0`. Treat the release PR as the `v2.6.0` release commit: after merge, create the signed annotated tag and publish verified assets in the same release window before announcement or support handoff.

1. Merge or directly push the release commit to `main` as Matt. PRs are recommended for reviewability but do not require another approver.
2. Complete full-history secret/customer-data scanning, then explicitly make the repository public. Enable public security features, the Matt-only bypass, branch rules, the release environment, and `RELEASE_SSH_ALLOWED_SIGNERS` in GitHub repository settings.
3. Produce one successful push-triggered `ci.yml` run for the exact release commit. Its five-minute `pr-gate` must complete the fast correctness gate and synthetic scale.
4. Dispatch `benchmark.yml` against that same `main` commit and require `full-evidence-gate` to pass. It re-runs the full local qualification plus the required external support matrix; scheduled benchmark runs cannot substitute for this dispatch.
5. Use Matt's existing passphrase-protected `~/.ssh/id_ed25519` key to create signed annotated `v2.6.0`. Do not add another key or change global Git configuration. The tag workflow independently verifies both exact-SHA runs before publishing.
6. Confirm the workflow publishes the exact stable tag and passes Linux, macOS ARM/x86, and Windows packaging and consumer smoke with checksums, SBOMs, and attestations.
7. Perform a clean-machine installation and one credential-free scan from the published package.
8. Never replace an exact release. Publish post-GA fixes as `2.6.1` and move `v2` only after verification.

Strict qualification requires a clean worktree, the exact signed tag at `HEAD`, `mdbook`, and `cargo-deny`. Development skip flags cannot create release evidence. Publication remains disabled outside GitHub Actions.

## Public repository controls

The tracked [allowed-signers file](../../../.github/release_allowed_signers) contains only Matt's existing public key. Its expected fingerprint is `SHA256:uiM1q8pDCkb7iW+6sNTblHdSYh4h0XUocIFIsUu8gGc`. Release controls require the Matt-only `release-owners` bypass team, enabled security features, protected release environment, and named branch rulesets.

## GA signing and qualification

After the exact-SHA main push and manually dispatched benchmark runs succeed, create and verify the stable tag. The tag command prompts to unlock the existing key when necessary:

```bash
git -c gpg.format=ssh \
  -c user.signingkey="$HOME/.ssh/id_ed25519" \
  tag -s -m "Costguard v2.6.0" v2.6.0

git -c gpg.format=ssh \
  -c gpg.ssh.allowedSignersFile=.github/release_allowed_signers \
  verify-tag v2.6.0

git push origin refs/tags/v2.6.0
```

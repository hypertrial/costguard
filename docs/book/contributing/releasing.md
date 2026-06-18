# Release checklist

GitHub Actions is the sole publication authority. Local scripts qualify code and recover deterministic packages but cannot publish.

Matt (`mattfaltyn`) is the sole release owner. The `release-owners` team contains only Matt and bypasses PR, review, and required-check rules, so Matt may self-merge or push directly to `main`. Other contributors require one approval plus successful `pr-gate`, `scale`, and `costguard` checks. Force pushes and default-branch deletion have no bypass. A direct push receives CI feedback after entering `main`; fix failures with a new commit, never by rewriting history.

1. Merge or directly push the release commit to `main` as Matt. PRs are recommended for reviewability but do not require another approver.
2. Complete full-history secret/customer-data scanning, then explicitly make the repository public. Enable public security features, the Matt-only bypass, branch rules, the release environment, and `RELEASE_SSH_ALLOWED_SIGNERS` in GitHub repository settings.
3. Produce one successful push-triggered `ci.yml` run for the exact release commit. The run must complete `pr-gate`, `scale`, `spellbook-smoke`, and `nba-monte-carlo-smoke`; the release workflow enforces this by commit SHA.
4. Use Matt's existing passphrase-protected `~/.ssh/id_ed25519` key to create signed annotated `v2.4.0`. Do not add another key or change global Git configuration.
5. Confirm the workflow publishes the exact stable tag and passes Linux, macOS ARM/x86, and Windows packaging and consumer smoke with checksums, SBOMs, and attestations.
6. Perform a clean-machine installation and one credential-free scan from the published package.
7. Never replace an exact release. Publish post-GA fixes as `2.0.1` and move `v2` only after verification.

Strict qualification requires a clean worktree, the exact signed tag at `HEAD`, `mdbook`, and `cargo-deny`. Development skip flags cannot create release evidence. Publication remains disabled outside GitHub Actions.

## Public repository controls

The tracked [allowed-signers file](../../../.github/release_allowed_signers) contains only Matt's existing public key. Its expected fingerprint is `SHA256:uiM1q8pDCkb7iW+6sNTblHdSYh4h0XUocIFIsUu8gGc`. Release controls require the Matt-only `release-owners` bypass team, enabled security features, protected release environment, and named branch rulesets.

## GA signing and qualification

After the exact-SHA main push run succeeds, create and verify the stable tag. The tag command prompts to unlock the existing key when necessary:

```bash
git -c gpg.format=ssh \
  -c user.signingkey="$HOME/.ssh/id_ed25519" \
  tag -s -m "Costguard v2.4.0" v2.4.0

git -c gpg.format=ssh \
  -c gpg.ssh.allowedSignersFile=.github/release_allowed_signers \
  verify-tag v2.4.0

git push origin refs/tags/v2.4.0
```

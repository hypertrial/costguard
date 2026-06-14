# Enterprise operations reference

Enterprise GA is Git-native technical readiness. Costguard does not provide a hosted control plane, SSO, commercial SLA, procurement workflow, telemetry, or warehouse connectivity. Cost estimates are advisory.

## Reference Action configuration

Pin an exact stable release in production:

```yaml
- uses: actions/checkout@v6
  with:
    fetch-depth: 0
- run: dbt compile --target ci
- uses: hypertrial/costguard/.github/actions/costguard@v2.1.0
  with:
    base: origin/main
    warehouse: snowflake
    manifest: target/manifest.json
    analysis-policy: strict
    fail-on: high
    min-confidence: high
    format: sarif
    baseline: .costguard/baseline.json
    policy: .costguard/policy.signed.json
    trust-store: .costguard/trust.json
    policy-organization: acme
    policy-team: data-platform
    policy-repository: acme/warehouse
```

Protect `.costguard/`, workflow files, and baseline changes with CODEOWNERS and required review. Retain the exact Action tag in audit evidence.

## 2.1 migration workflow

Costguard 2.1 requires **atomic deployment** of the binary, policy v2, and baseline v3. Partial rollout (new binary with old baseline or policy) fails closed with migration guidance.

1. On a branch from default, run `dbt compile` with production settings.
2. Generate an identity map with the same warehouse, manifest, and signed-policy inputs used in CI:

```bash
costguard baseline identity-map-v2 .costguard/identity-map.json \
  --warehouse snowflake \
  --manifest target/manifest.json \
  --policy .costguard/policy.signed.json \
  --trust-store .costguard/trust.json \
  --policy-organization acme \
  --policy-repository acme/warehouse
```

3. Migrate artifacts:

```bash
costguard baseline migrate-v2 .costguard/baseline.json .costguard/identity-map.json .costguard/baseline.json \
  --policy .costguard/policy.signed.json \
  --trust-store .costguard/trust.json

costguard policy migrate-v1 .costguard/policy.json .costguard/identity-map.json .costguard/policy.v2.json \
  --version 2026.06.14 \
  --issued-at 2026-06-14T00:00:00Z
costguard policy sign .costguard/policy.v2.json root-2026.private.json .costguard/policy.signed.json
```

4. Bump the Action pin to `@v2.1.0`, commit baseline v3 and signed policy v2 together, and merge in one release window.
5. Re-run a full strict scan on default branch and confirm PR checks pass.

Retain pre-migration baseline v2 and policy v1 artifacts until rollback is no longer required. See [Compatibility policy](../reference/compatibility.md) for rollback to `@v2.0.0`.

## Baseline rollout

1. Run a full strict scan on the default branch and resolve parse or metadata failures.
2. Generate a finding baseline bound to the active signed-policy digest (baseline v3 with `identity_scheme: "semantic-v1"`).
3. Review baseline entries as accepted existing debt, then commit under CODEOWNERS.
4. Enable changed-file PR enforcement so only new findings fail.
5. Remove baseline entries as debt is fixed. Never regenerate the entire baseline merely to make a failing PR pass.

Policy can forbid repository baselines. A baseline whose policy digest differs from the active policy fails closed.

## Key rotation

1. Generate a new offline signing key and add only its public key to the trust store with a future-valid overlap window.
2. Merge and deploy the trust-store update before signing a new policy bundle.
3. Sign and verify the policy with the new key, then update repositories to the new pinned bundle.
4. After migration, expire the old key. Mark it revoked immediately if compromise is suspected.
5. Treat unknown, revoked, expired, malformed, and invalid signatures as blocking failures; do not bypass strict mode.

Private keys must not enter application repositories, CI logs, build artifacts, or developer backups outside the approved signing system.

## Exception lifecycle

Every exception needs an immutable ID, finding or rule scope, repository/path scope, owner, reason, ticket, approver, creation time, and expiry. Require review by the policy owner, keep duration short, and link remediation work. Expired exceptions stop suppressing and make analysis incomplete. Extend by reviewed replacement, not by editing historical evidence.

## SARIF retention

Retain SARIF with the source commit, exact Costguard version, policy bundle digest, trust-store revision, baseline digest, and workflow run URL. A minimum retention period should match the organization's code-scanning and change-management policy. Result properties include finding/evidence IDs, confidence, enforcement outcome, policy provenance, applied exception, and advisory cost estimate.

## Rollback

Rollback means pinning the last known-good exact `2.x` release and its compatible policy bundle and baseline. For 2.1 rollbacks, restore `@v2.0.0` with saved policy v1 and baseline v2 artifacts. Never retag or replace an exact release. Keep `v2` out of regulated workflows when immutable behavior is required. If a policy rollout fails, revert the policy artifact and trust-store commit together where necessary; do not disable attestation or strict verification. Re-run a full scan after rollback and retain both failed and recovery evidence.

# Compatibility policy

Costguard follows semantic versioning for the `2.x` line. `v0.1.0` was the only earlier public release; internal `1.x` development builds were not public releases.

## Stable contracts

- CLI exit codes and existing command/flag meanings
- JSON `schema_version: 3` fields and meanings
- `cost_estimate.p50_usd_per_month` on findings represents **estimated savings** (v2); use `model_monthly_p50_usd` for model baseline when present
- `SQLCOST###` identifiers; an identifier is never reused for another rule
- suppression syntax and matching behavior
- accepted configuration keys and precedence
- exact release tags and their attached assets

Minor releases may add optional JSON fields, configuration keys, commands, flags, rules, or diagnostics. Consumers must ignore unknown additive JSON fields. Removing or renaming stable fields, changing an existing rule to detect a materially different condition, or reusing a rule ID requires a major release.

The moving `v2` Action tag may advance only to compatible stable `2.x` releases. Use exact `v2.1.0` when immutable behavior is required.

Preview warehouse dialects may receive parser and rule refinements in minor releases. Production-supported dialects retain the stable contracts above.

## Version 2.1 identity and artifact schemas

Costguard 2.1 introduces **semantic finding identity** (`identity_scheme = "semantic-v1"`). Finding IDs and evidence keys are derived from rule, path, and canonical evidence fields—not ordinal `occurrence:N` positions—so IDs survive formatting and line movement.

| Artifact | Schema | Required field |
| --- | --- | --- |
| Finding baseline | v3 | `identity_scheme: "semantic-v1"` |
| Signed policy document | v2 | `identity_scheme: "semantic-v1"` |
| JSON scan output | v3 | optional top-level `identity_scheme` |

At scan time, Costguard **rejects** baseline v2 and policy v1 with an exact migration-command hint. Do not expect partial compatibility: the 2.1 binary, policy v2, and baseline v3 must be deployed together.

Use `costguard baseline migrate-v1` only for baselines produced by internal pre-v2 builds (legacy v1 format without stable finding IDs).

## Migration to 2.1

Deploy atomically in one change set:

1. Pin the Costguard binary or Action to `@v2.1.0` (or newer compatible `2.x`).
2. Generate an identity map from the same project, warehouse, manifest, and signed-policy context used in production.
3. Migrate the baseline and policy bundle with that map.
4. Commit and roll out the new baseline v3 and policy v2 together.

```bash
# 1. Identity map (run on default branch with production scan settings)
costguard baseline identity-map-v2 identity-map.json \
  --warehouse snowflake \
  --manifest target/manifest.json \
  --policy .costguard/policy.signed.json \
  --trust-store .costguard/trust.json \
  --policy-organization acme \
  --policy-repository acme/warehouse

# 2. Baseline v2 → v3
costguard baseline migrate-v2 .costguard/baseline.json identity-map.json .costguard/baseline.v3.json \
  --policy .costguard/policy.signed.json \
  --trust-store .costguard/trust.json

# 3. Policy v1 → v2 (when upgrading signed policy)
costguard policy migrate-v1 .costguard/policy.json identity-map.json .costguard/policy.v2.json \
  --version 2026.06.14 \
  --issued-at 2026-06-14T00:00:00Z
# Sign the migrated document before use in CI
```

Migration commands fail closed (exit code `2`) on missing map entries, platform mismatch, or invalid artifacts. Review map coverage before merging.

## Rollback to 2.0

Retain the pre-migration **policy v1** and **baseline v2** artifacts (and their digests) before deploying 2.1. To roll back:

1. Pin the Action or binary to `@v2.0.0`.
2. Restore the saved baseline v2 and policy v1 bundle (and trust store if changed).
3. Re-run a full scan and retain rollback evidence.

Never retag or replace an exact release. Keep `@v2` out of regulated workflows when immutable behavior is required.

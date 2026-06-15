# Compatibility policy

Costguard follows semantic versioning for the `2.x` line. `v0.1.0` was the only earlier public release; internal `1.x` development builds were not public releases.

## Stable contracts

- CLI exit codes and existing command/flag meanings
- JSON `schema_version: 4` fields and meanings (v3 consumers may ignore new structured `cost` blocks)
- `cost_estimate.p50_usd_per_month` on findings represents **estimated savings** (v2); use `model_monthly_p50_usd` for model baseline when present
- `SQLCOST###` identifiers; an identifier is never reused for another rule
- suppression syntax and matching behavior
- accepted configuration keys and precedence
- exact release tags and their attached assets

Minor releases may add optional JSON fields, configuration keys, commands, flags, rules, or diagnostics. Consumers must ignore unknown additive JSON fields. Removing or renaming stable fields, changing an existing rule to detect a materially different condition, or reusing a rule ID requires a major release.

The moving `v2` Action tag may advance only to compatible stable `2.x` releases. Use exact `v2.2.0` when immutable behavior is required.

Preview warehouse dialects may receive parser and rule refinements in minor releases. Production-supported dialects retain the stable contracts above.

## Version 2.1 identity and artifact schemas

Costguard 2.1 uses **semantic finding identity** (`identity_scheme = "semantic-v1"`). Finding IDs and evidence keys are derived from rule, path, and canonical evidence fields—not ordinal `occurrence:N` positions—so IDs survive formatting and line movement.

| Artifact | Schema | Required field |
| --- | --- | --- |
| Finding baseline | v3 | `identity_scheme: "semantic-v1"` |
| Signed policy document | v2 | `identity_scheme: "semantic-v1"` |
| JSON scan output | v3 | optional top-level `identity_scheme` |

At scan time, Costguard **rejects** baseline v2, policy v1, and any artifact without `identity_scheme: "semantic-v1"`. Deploy the 2.1 binary with baseline v3 and policy v2 together.

Generate baselines with `costguard scan --write-baseline`. Write policy v2 documents with `costguard policy compile` and sign before use in CI.

## Rollback to 2.0

Retain the pre-upgrade **policy v1** and **baseline v2** artifacts (and their digests) before deploying 2.1. To roll back:

1. Pin the Action or binary to `@v2.0.0`.
2. Restore the saved baseline v2 and policy v1 bundle (and trust store if changed).
3. Re-run a full scan and retain rollback evidence.

Never retag or replace an exact release. Keep `@v2` out of regulated workflows when immutable behavior is required.

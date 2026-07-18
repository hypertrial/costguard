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

The v2.6 correctness repair keeps schema v4 and restores the documented units: `monthly_p*`, `annual_p50`, `project_p*_usd`, and `savings_p*_usd` are monetary only and are `null` when USD is unavailable. Full-project relative volume uses additive `CostFigure.gb_months_p50`; additive coverage fields report the USD-mapped model count and fraction. Unpriced byte values are never serialized as dollars.

The v2.6 hardening remains source-compatible for existing public Rust configuration and cost structs. `ResolvedScanRequest` and `receipt_trend` are additive APIs; legacy `scan`, `doctor`, `load_config`, and configuration structs retain their existing signatures and use the 2 GiB base-replay default. Receipt comparison accepts tolerant schema-v4 JSON, requires `diagnostics`, and ignores unknown fields.

First-class Rocky support is additive. JSON remains schema v4 and PR receipts remain version 2: consumers must tolerate optional `changed_model_details[].framework`, `recommended_commands`, and `rocky_artifact_integrity`. Existing `recommended_dbt_command`, `Project.dbt`, dbt public APIs, public cost entry points, and scanner file shapes remain available.

The sealed Rocky envelope sets `schema_version: 1`. Readers tolerate additional Rocky compile fields and unknown future strategies, but reject a different envelope schema, missing expanded SQL, ambiguous source mappings, commit/input mismatches, and oversized artifacts.

The moving `v2` Action tag may advance only to compatible stable `2.x` releases. Use exact `v2.6.0` when immutable behavior is required.

Preview warehouse dialects may receive parser and rule refinements in minor releases. Production-supported dialects retain the stable contracts above.

## Version 2.1 identity and artifact schemas

Costguard 2.1 uses **semantic finding identity** (`identity_scheme = "semantic-v1"`). Finding IDs and evidence keys are derived from rule, path, and canonical evidence fields—not ordinal `occurrence:N` positions—so IDs survive formatting and line movement.

| Artifact | Schema | Required field |
| --- | --- | --- |
| Finding baseline | v3 | `identity_scheme: "semantic-v1"` |
| Signed policy document | v2 | `identity_scheme: "semantic-v1"` |
| JSON scan output | v4 | optional top-level `identity_scheme` |
| Sealed Rocky artifact | v1 | `schema_version: 1`, `framework: "rocky"` |

At scan time, Costguard **rejects** baseline v2, policy v1, and any artifact without `identity_scheme: "semantic-v1"`. Deploy the 2.1 binary with baseline v3 and policy v2 together.

Generate baselines with `costguard scan --write-baseline`. Write policy v2 documents with `costguard policy compile` and sign before use in CI.

## Rollback to 2.0

Retain the pre-upgrade **policy v1** and **baseline v2** artifacts (and their digests) before deploying 2.1. To roll back:

1. Pin the Action or binary to the saved `v2.0.0` release.
2. Restore the saved baseline v2 and policy v1 bundle (and trust store if changed).
3. Re-run a full scan and retain rollback evidence.

Never retag or replace an exact release. Keep `@v2` out of regulated workflows when immutable behavior is required.

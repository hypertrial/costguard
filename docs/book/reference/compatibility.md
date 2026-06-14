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

The moving `v2` Action tag may advance only to compatible stable `2.x` releases. Use exact `v2.0.0-rc.2` during RC validation and exact `v2.0.0` after GA when immutable behavior is required. RC tags never move `v2`. Version 2 intentionally does not retain internal v1 runtime aliases or output fields; use `costguard baseline migrate-v1` only for baselines produced by internal pre-v2 builds.

Preview warehouse dialects may receive parser and rule refinements in minor releases. Production-supported dialects retain the stable contracts above.

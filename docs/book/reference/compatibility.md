# Compatibility policy

Costguard follows semantic versioning from `1.0.0` onward.

## Stable contracts

- CLI exit codes and existing command/flag meanings
- JSON `schema_version: 1` fields and meanings
- `SQLCOST###` identifiers; an identifier is never reused for another rule
- suppression syntax and matching behavior
- accepted configuration keys and precedence
- exact release tags and their attached assets

Minor releases may add optional JSON fields, configuration keys, commands, flags, rules, or diagnostics. Consumers must ignore unknown additive JSON fields. Removing or renaming stable fields, changing an existing rule to detect a materially different condition, or reusing a rule ID requires a major release.

The moving `v1` Action tag may advance only to compatible `1.x` releases. Use an exact tag such as `v1.1.0` when immutable behavior is required.

Preview warehouse dialects may receive parser and rule refinements in minor releases. Production-supported dialects retain the stable contracts above.

# Signed policy and exceptions

Git-native governance uses the same local binary. Policy and exceptions are versioned git artifacts—no hosted control plane required.

Organization policy is authored as strict TOML, compiled to canonical JSON, and signed with Ed25519 in an offline or CI signing environment. Distribute signed bundles as pinned git artifacts (for example `policy.signed.json` at a tagged release). The Action and CLI verify bundles against a committed public trust store.

```bash
costguard policy keygen root-2026 --private-key root-2026.private.json --trust-store .costguard/trust.json
costguard policy compile policy.toml policy.json
costguard policy sign policy.json root-2026.private.json policy.signed.json
costguard policy verify policy.signed.json .costguard/trust.json
costguard policy resolve policy.signed.json .costguard/trust.json \
  --organization acme --repository acme/warehouse --path models/marts/orders.sql
```

Key generation never overwrites by default. Use `--force` only for deliberate rotation into existing regular files. Private-key and trust-store destinations must differ; symlinks, directories, and special files are always rejected. Outputs are serialized first and atomically persisted from same-directory temporary files; Unix private keys are created with `0600` permissions before key bytes are written.

Commit only the public trust store (`.costguard/trust.json`). Protect policy, exception, and trust-store changes with CODEOWNERS and branch protection rules. Rotate keys by overlapping validity windows, publish the new public key before signing with it, and mark compromised keys revoked. Unknown, revoked, expired, malformed, non-canonical, or invalid signatures fail closed.

## Policy schema v2

Compiled policy documents require:

| Field | Value |
| --- | --- |
| `schema_version` | `2` |
| `identity_scheme` | `"semantic-v1"` |

Policy v1 and any document without `identity_scheme: "semantic-v1"` are rejected at scan time. Author new policy with `costguard policy compile` and sign before deployment.

Exception entries reference findings by semantic `finding_id`.

Resolution order is organization, team, repository, then path. Equal-specificity conflicts at the same priority are configuration errors. Central policy decides whether local severity changes, CLI overrides, inline suppressions, repository baselines, and local waivers are permitted. Local `[[waivers]]` are rejected under a signed policy unless `permissions.allow_local_waivers = true`.

Every exception requires an immutable ID, finding or rule scope, repository and path globs, owner, reason, ticket URL, approver, creation time, and expiry. Expired exceptions stop suppressing findings and make analysis incomplete.

Repository-local waivers use the same audit metadata but match exactly one `finding_id` or `rule_id` plus a repository-relative path glob. Active waivers mark findings `excepted`; every expired local waiver makes analysis incomplete with `expired_waiver`.

Use signed policy bundles via `[policy]` in `costguard.toml` or with `--policy` and `--trust-store` on `scan` and `pr`. For highly regulated teams, signed bundles provide a defensible audit trail while remaining distributable as pinned git artifacts.

See [Configuration](../reference/configuration.md#policy) for the committed config schema.

See [Enterprise operations](../getting-started/enterprise-operations.md) for rotation, exception, retention, baseline, and rollback procedures, and the [threat model](../security/threat-model.md) for trust boundaries and residual risks.

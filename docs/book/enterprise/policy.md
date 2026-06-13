# Signed policy and exceptions

Organization policy is authored as strict TOML, compiled to canonical JSON, and signed outside the server with Ed25519. The private key must remain in an offline or CI signing environment.

```bash
costguard policy keygen root-2026 --private-key root-2026.private.json --trust-store .costguard/trust.json
costguard policy compile policy.toml policy.json
costguard policy sign policy.json root-2026.private.json policy.signed.json
costguard policy verify policy.signed.json .costguard/trust.json
costguard policy resolve policy.signed.json .costguard/trust.json \
  --organization acme --repository acme/warehouse --path models/marts/orders.sql
```

Commit only the public trust store. Rotate keys by overlapping validity windows, publish the new public key before signing with it, and mark compromised keys revoked. Unknown, revoked, expired, malformed, non-canonical, or invalid signatures fail closed.

Resolution order is organization, team, repository, then path. Equal-specificity conflicts at the same priority are configuration errors. Central policy decides whether local severity changes, CLI overrides, inline suppressions, and repository baselines are permitted.

Every exception requires an immutable ID, finding or rule scope, repository and path globs, owner, reason, ticket URL, approver, creation time, and expiry. Expired exceptions stop suppressing findings and make analysis incomplete.

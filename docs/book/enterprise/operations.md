# Operations, privacy, and recovery

## Data boundary

The control plane accepts repository identity, commit and pull request metadata, policy digest, finding metadata, parse completeness, aggregate metrics, estimated cost metadata, and normalized offline cost observations. Protocol schemas deny unknown fields and contain no source-content properties.

Ordinary local scans perform no network activity. Policy retrieval and scan publication occur only when the Action `publication-mode` is `optional` or `required`. Optional publication failures are warnings; required failures return runtime exit code `3`.

## Backup and restore

Use PostgreSQL-native backups. A minimum logical backup is:

```bash
pg_dump --format=custom --no-owner --file=costguard.dump "$DATABASE_URL"
pg_restore --clean --if-exists --no-owner --dbname="$RESTORE_DATABASE_URL" costguard.dump
```

Test restores on every release. Preserve audit rows, policy bundles, trust stores, repository identities, token hashes, findings, scans, and cost observations together. Tokens cannot be recovered from hashes; issue replacements after a suspected secret loss.

## Security controls

- Service tokens are organization-scoped, expire, and store only a prefix and Argon2id hash.
- UI sessions use short-lived Secure, HttpOnly, SameSite cookies.
- State-changing browser forms require CSRF tokens.
- Audit events are append-only and hash chained.
- PostgreSQL foreign keys, explicit organization filters, and RLS provide layered tenant isolation.
- Raw GitHub webhook bodies are never retained.

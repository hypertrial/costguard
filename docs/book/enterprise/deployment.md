# Self-hosted deployment

Costguard 2.0 deploys as one non-root server image plus PostgreSQL. Scans continue to run on customer CI runners; the server never clones repositories and never receives SQL, YAML, Python, manifests, snippets, or warehouse credentials.

## Compose quick start

1. Copy `.env.example` to a secret-managed environment file and replace both required secrets.
2. Put a TLS reverse proxy in front of the server. Compose binds the application to loopback by default.
3. Start the stack with `docker compose up -d --build`.
4. Wait for `docker compose ps` to report both services healthy.
5. Bootstrap the first owner once:

```bash
curl --fail --request POST https://costguard.example.com/api/v1/bootstrap \
  --header "X-Costguard-Bootstrap-Secret: ${COSTGUARD_BOOTSTRAP_SECRET}" \
  --header 'Content-Type: application/json' \
  --data '{"organization_slug":"acme","organization_name":"Acme","owner_email":"owner@example.com","owner_name":"Owner"}'
```

The response contains the initial organization token exactly once. Store it in a secret manager, then remove the bootstrap secret from the running service configuration.

## Production requirements

- Terminate TLS at a maintained reverse proxy and forward only trusted request headers.
- Back up the PostgreSQL volume using native `pg_dump` or physical backups.
- Restrict database access to the application network.
- Mount the GitHub App private key as a secret where the platform supports mounted secrets.
- Monitor `/readyz`, `/metrics`, webhook failures, queued checks, expired exceptions, and PostgreSQL storage.
- Keep the image immutable and run it as UID/GID `10001`.

SQLite and Helm are not supported in 2.0.

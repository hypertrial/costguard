# GitHub App

Request only these repository permissions:

- Metadata: read
- Pull requests: read
- Checks: write

Subscribe to installation, repository selection, pull request, and check action events. Configure the webhook URL as `/webhooks/github` and use a high-entropy webhook secret.

Costguard verifies `X-Hub-Signature-256` over the untouched request body, deduplicates `X-GitHub-Delivery`, and stores only delivery metadata and a payload hash. Pull request events create one queued check per repository, head SHA, and policy digest. The runner Action fetches the signed policy, verifies it against `.costguard/trust.json`, scans locally, and uploads a metadata-only envelope.

For GitHub Enterprise Server, set `GITHUB_API_URL` and `GITHUB_WEB_URL`. Installation tokens are short-lived and cached until shortly before expiry. Check annotations are sent in batches of 50.

Action example:

```yaml
- uses: hypertrial/costguard/.github/actions/costguard@v2
  with:
    server-url: https://costguard.example.com
    publication-mode: required
    organization: acme
    trust-store: .costguard/trust.json
    token: ${{ secrets.COSTGUARD_TOKEN }}
    warehouse: snowflake
```

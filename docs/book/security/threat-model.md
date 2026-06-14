# Threat model

## Scope and security goals

Costguard analyzes untrusted repository content locally or in CI without warehouse credentials or telemetry. The primary goals are deterministic analysis, containment to the checked-out workspace, fail-closed signed-policy enforcement, authenticated release installation, and auditable findings. Costguard does not execute scanned SQL against a warehouse.

## Trust boundaries

| Boundary | Untrusted input | Control |
| --- | --- | --- |
| Repository scan | SQL, YAML, Python, manifests, config, baselines | Parsers, size limits, no SQL execution, explicit paths |
| Filesystem | Symlinks, archive entries, output paths | Workspace-relative operation, safe archive layout validation, data-only extraction |
| Git | Base refs, changed paths, history | Argument-based subprocess calls, required history, no shell interpolation |
| Release download | Archives, checksums, attestations | HTTPS timeout/retry, exact checksum filename, SHA-256, producer-bound attestation |
| Signed policy | Bundle, trust store, scopes, exceptions | Canonical Ed25519 verification, validity/revocation checks, conflict rejection, fail closed |
| Offline cost imports | Catalog and query-history files | Local parsing only, advisory output, no warehouse connection |
| Artifact schemas | Baseline v2, policy v1 | Rejected at scan time; baseline v3 and policy v2 with semantic-v1 required |

## Threats and mitigations

### Malicious repository content

Repository files may be crafted to trigger parser failures, excessive work, misleading paths, or diagnostic injection. Costguard treats content as data, escapes CI/Markdown output, records parse failures, applies configured file-size limits, and uses strict analysis mode to reject incomplete coverage. Scale and Spellbook gates bound expected runtime and memory, but deliberately adversarial parser inputs remain a residual denial-of-service risk.

### Filesystem escape and archive traversal

Scans should be rooted in the configured project. Release archives are accepted only when their member layout exactly matches the expected binary, members are regular files, and extraction uses safe data filtering. Unexpected absolute paths, traversal components, links, or extra members are rejected before execution.

### Git argument and changed-file manipulation

Git refs and paths are passed as subprocess arguments, not concatenated shell commands. CI must check out full history for PR comparison. A malicious base selection can change scan scope, so protected workflows must set the base centrally and prevent untrusted workflow edits.

### Release substitution

The Action downloads bounded-size release assets with a 30-second timeout and three attempts. It requires an exact sidecar checksum filename and digest, rejects unsafe archives, and verifies GitHub artifact attestations against producer repository `hypertrial/costguard`. Consumer repository identity is never used as the producer. Exact releases are immutable; RC tags do not move `v2`.

### Policy bypass

Unknown, tampered, expired, or revoked signing keys fail closed. Scope resolution proceeds organization, team, repository, then path; equal-specificity conflicts at equal priority are errors. Policy controls local overrides, inline suppression, and repository baselines. Expired exceptions no longer suppress findings and produce an analysis violation.

Policy v1 and baseline v2 are rejected at scan time. Only baseline v3 and policy v2 with `identity_scheme: "semantic-v1"` are accepted.

### Semantic identity tampering

Finding IDs under `semantic-v1` are derived from rule, path, and canonical evidence fields—not line numbers. Baseline and exception entries must match computed semantic IDs; operators cannot grandfather findings by editing line offsets alone. Duplicate semantic findings collapse to one diagnostic, reducing ID-splitting evasion.

### Sensitive offline cost data

Catalog and query-history imports can contain operational metadata. Costguard reads them locally and emits advisory aggregates, but operators remain responsible for minimizing fields, access control, artifact retention, and excluding raw imports from public repositories. Costguard has no telemetry or warehouse connectivity.

### CI and supply-chain administration

Compromise of GitHub organization owners, release environments, signing keys, or required-check configuration can bypass repository controls. Mitigations include least-privilege workflow permissions, SHA-pinned Actions, protected release environments, signed annotated tags, private vulnerability reporting, dependency alerts, push protection, branch rules, and full-history secret/customer-data scanning before public launch.

## Residual risks

- Static heuristics can produce false positives or miss costly behavior; precision gates reduce but do not eliminate this risk.
- Cost estimates are prioritization signals, not billing-grade calculations.
- A compromised trusted policy signing key can authorize malicious policy until revoked and distributed.
- CI administrators and repository owners remain privileged actors.
- Very large or adversarial source files can consume CI resources within platform limits.
- Preview dialects can change behavior in compatible minor releases and are not enterprise production-supported.

Report suspected vulnerabilities through the private process in [`SECURITY.md`](../../../SECURITY.md).

# Threat model

## Scope and security goals

Costguard analyzes untrusted repository content locally or in CI without warehouse credentials or telemetry. The primary goals are deterministic analysis, containment to the checked-out workspace, fail-closed signed-policy enforcement, authenticated release installation, and auditable findings. Costguard does not execute scanned SQL against a warehouse.

## Trust boundaries

| Boundary | Untrusted input | Control |
| --- | --- | --- |
| Repository scan | SQL, YAML, Python, manifests, config, baselines | Parsers, size limits, no SQL execution, explicit paths |
| Filesystem | Symlinks, archive entries, output paths | Workspace-relative operation, safe archive layout validation, data-only extraction |
| Git | Base refs, changed paths, history | NUL-safe rename-aware discovery, immutable resolved base commit, per-blob and aggregate preflight, exact-length streaming, no shell interpolation |
| Rocky artifact | Compile JSON, source map, sealed inputs | Schema and size limits, tracked regular-file inputs, source-map completeness, SHA-256 verification against the working tree and immutable Git commits |
| Release download | Archives, checksums, attestations | 64 MiB archive and 4 KiB sidecar limits, HTTPS timeout/retry, exact checksum filename, SHA-256, producer-bound attestation |
| Signed policy | Bundle, trust store, scopes, exceptions | Canonical Ed25519 verification, validity/revocation checks, conflict rejection, fail closed |
| Offline cost imports | Catalog and query-history files | Local parsing only, advisory output, no warehouse connection |
| Artifact schemas | Baseline v2, policy v1 | Rejected at scan time; baseline v3 and policy v2 with semantic-v1 required |

## Threats and mitigations

### Malicious repository content

Repository files may be crafted to trigger parser failures, excessive work, misleading paths, or diagnostic injection. Costguard treats content as data, escapes CI/Markdown output, records parse failures, applies the 5 MiB default source limit, 512 MiB manifest limit, and 2 GiB aggregate base-snapshot limit, and uses strict analysis mode to reject incomplete coverage. Base replay first parses the optional Rocky envelope, then resolves one deduplicated request containing every immutable SQL/YAML/Python/config file, manifest, Rocky sealed input, and Rocky model source. `git cat-file --batch-check -Z` validates existence, blob type, the strictest applicable individual limit, and the complete unique-byte total before any content process starts. Approved bytes are streamed once with exact blob lengths and reused by dbt metadata, Rocky verification, and analysis; an explicit local base manifest consumes the same budget once. Oversized combined comparisons now fail closed even when their former phase-by-phase reads would each have fit, without producing partial deltas. Scale and Spellbook gates bound expected runtime and memory, but deliberately adversarial parser inputs within configured limits remain a residual denial-of-service risk.

Sealed Rocky envelopes are untrusted repository or CI inputs. Capture accepts only tracked, clean regular files inside the project root, rejects traversal and symlink escapes, and maps every compiled model to exactly one sealed source. Analysis uses expanded SQL only after the envelope commit and every input hash match both the current filesystem and immutable Git `HEAD`; base verification reads the same inputs from the resolved comparison commit. Costguard does not establish that the Rocky compiler itself is trustworthy, so CI remains responsible for pinning and securing the Rocky toolchain that produces compile JSON.

### Filesystem escape and archive traversal

Scans should be rooted in the configured project. Release archives are accepted only when their member layout exactly matches the expected binary, members are regular files, and extraction uses safe data filtering. Unexpected absolute paths, traversal components, links, or extra members are rejected before execution.

### Git argument and changed-file manipulation

Git refs and paths are passed as subprocess arguments, not concatenated shell commands. Changed paths, rename pairs, and batch object requests use NUL delimiters. Costguard resolves the merge-base commit once, approves the complete shared base snapshot before reading content, and reads only from that immutable commit. Head and base content acquisition differ, but both feed the same framework assembler so ownership, provenance, and analysis selection cannot drift between comparison sides. CI must check out full history for PR comparison. `doctor` parses workflow YAML and requires one complete Costguard job, so comments and unrelated jobs cannot spoof readiness. A malicious base selection can still change scan scope, so protected workflows must set the base centrally and prevent untrusted workflow edits.

### Release substitution

The Action limits release archives to `67108864` bytes (64 MiB) and checksum sidecars to `4096` bytes. It rejects oversized `Content-Length` values before writing, counts streamed bytes when headers are absent or dishonest, removes partial files on every failure, and does not retry deterministic oversize failures. Transient I/O retains the 30-second timeout and three attempts. The Action also requires an exact sidecar checksum filename and digest, rejects unsafe archives, and verifies GitHub artifact attestations against producer repository `hypertrial/costguard`. Consumer repository identity is never used as the producer. Exact releases are immutable; RC tags do not move `v2`.

### Policy bypass

Unknown, tampered, expired, or revoked signing keys fail closed. Scope resolution proceeds organization, team, repository, then path; equal-specificity conflicts at equal priority are errors. Policy controls local overrides, inline suppression, and repository baselines. Expired exceptions no longer suppress findings and produce an analysis violation.

Policy v1 and baseline v2 are rejected at scan time. Only baseline v3 and policy v2 with `identity_scheme: "semantic-v1"` are accepted.

### Semantic identity tampering

Finding IDs under `semantic-v1` are derived from rule, path, and canonical evidence fields—not line numbers. Baseline and exception entries must match computed semantic IDs; operators cannot grandfather findings by editing line offsets alone. Duplicate semantic findings collapse to one diagnostic, reducing ID-splitting evasion.

### Sensitive offline cost data

Catalog and query-history imports can contain operational metadata. Costguard reads them locally and emits advisory aggregates, but operators remain responsible for minimizing fields, access control, artifact retention, and excluding raw imports from public repositories. Costguard has no telemetry or warehouse connectivity.

### CI and supply-chain administration

Compromise of GitHub organization owners, release environments, signing keys, or required-check configuration can bypass repository controls. Mitigations include least-privilege workflow permissions, SHA-pinned Actions, protected release environments, signed annotated tags, exact-SHA push CI plus independently dispatched benchmark qualification, private vulnerability reporting, dependency alerts, push protection, branch rules, and full-history secret/customer-data scanning before public launch.

## Residual risks

- Static heuristics can produce false positives or miss costly behavior; precision gates reduce but do not eliminate this risk.
- Cost estimates are prioritization signals, not billing-grade calculations.
- A compromised trusted policy signing key can authorize malicious policy until revoked and distributed.
- CI administrators and repository owners remain privileged actors.
- Very large or adversarial inputs can consume CI resources within configured source/manifest limits; raising those limits increases exposure.
- Preview dialects can change behavior in compatible minor releases and are not enterprise production-supported.

Report suspected vulnerabilities through the private process in [`SECURITY.md`](../../../SECURITY.md).

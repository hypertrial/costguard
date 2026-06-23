# Costguard vs SlowQL

> Comparison snapshot: SlowQL 1.6.7, commit [`cb6907d`](https://github.com/slowql/slowql/tree/cb6907dd12a55dd27038f1420f4114889d6e0dfd). Last verified: 2026-06-23. SlowQL evolves independently; recheck its [versioned source](https://github.com/slowql/slowql/blob/cb6907dd12a55dd27038f1420f4114889d6e0dfd/README.md) and [PyPI release](https://pypi.org/project/slowql/1.6.7/) before relying on this snapshot.

SlowQL finds SQL problems. Costguard governs dbt cost changes.

This is a product-boundary comparison, not a claim that one tool replaces the other. SlowQL 1.6.7 is substantially broader: its official release describes 282 rules across security, performance, reliability, compliance, cost, and quality, with schema inspection, custom rules, comparison mode, safe autofix, an LSP/VS Code path, and several report formats. Costguard deliberately does less general SQL analysis so it can make the dbt pull request—not the individual SQL file—the governed unit.

| Dimension | SlowQL 1.6.7 | Costguard |
| --- | --- | --- |
| Unit of analysis | SQL files/statements, optional schemas, cross-file context, and query comparison | Git base/head dbt change: changed models plus manifest, macros, sources, lineage, exposures, and project context |
| Breadth | Broad six-domain SQL analyzer with 282 rules, custom rules, schema validation, safe autofix, and editor support | 46 cost/performance-focused SQLCOST rules; no generic security/compliance analyzer, autofix, or LSP goal |
| dbt depth | Recognizes dbt model context as one input classification | Treats dbt state as the product core: manifest model identity, materializations, compiled SQL, refs/sources, transitive downstream nodes, exposures, tags, groups, owners, and recommended `dbt build --select` |
| Cost model | Cost is one analyzer dimension among several | Model-centric current/post-fix cost, introduced/avoided/net PR impact, efficiency/volume split, downstream blast-radius cost, A/B/C provenance, and mapped-spend coverage |
| Change semantics | Query comparison is available, while normal findings are file/statement diagnostics | Stable semantic finding IDs classify introduced, severity/cost-regressed, resolved, and unchanged findings; regression-only enforcement is a first-class gate |
| Governance | Failure thresholds, allow/deny and context filters, inline suppression, custom rules | Owners, CODEOWNERS, global/scoped gates, required-owner and blast-radius controls, baselines, expiring waivers, signed policy, and fail-closed identity handling |
| Evidence | Console/GitHub/SARIF plus JSON, HTML, and CSV exports | GitHub annotations, markdown decision summary, SARIF, JSON schema v4/receipt v2, trend comparison, policy provenance, gate reasons, finding delta, cost coverage, and reproducible benchmark ledger |
| Deployment | Python 3.11+ package with optional extras for interactive, LSP, and other capabilities | Single Rust binary and composite GitHub Action; scans local files without warehouse credentials, live queries, or a service |

## When each fits

Choose SlowQL when the main requirement is broad SQL quality and security coverage, schema checks, safe autofix, custom analyzer extension, or editor feedback across SQL beyond dbt.

Choose Costguard when the merge decision needs to answer:

- Did this PR introduce or regress an expensive dbt finding?
- What cost did it introduce, avoid, and add net after base/head comparison?
- Which downstream models and exposures inherit the blast radius?
- Who owns the changed models, which policy gate decided, and why?
- Can the exact decision evidence be retained and compared later?

The tools can be complementary: SlowQL can provide broad SQL diagnostics while Costguard owns the dbt cost-change gate and receipt. Avoid configuring both to block the same undifferentiated all-findings threshold; that discards Costguard's regression semantics and produces duplicate review noise.

## Verification notes

The SlowQL side of this page is based only on its official 1.6.7 release metadata and source at the pinned commit. Reverify the rule count, CLI, dbt handling, outputs, autofix, and LSP claims whenever the pinned comparison version changes. Costguard claims are validated by this repository's core PR replay/cost-impact tests, output and CLI tests, Action contract tests, docs checks, and release qualification described in [Benchmark evidence](benchmarks.md).

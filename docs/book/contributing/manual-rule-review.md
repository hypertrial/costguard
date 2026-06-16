# Manual rule review

Costguard’s 44 rules are validated on real dbt projects through **manual adjudication**: humans read compiled SQL, classify findings into buckets, and record TP/FP verdicts in machine-readable artifacts that CI enforces.

## When to use this

- Tuning a rule after external benchmark feedback
- Clearing unknown buckets from a census run
- Cost-prioritized triage on Spellbook (or another pinned repo)
- Onboarding reviewers before editing [`fp_registry.toml`](../../../tests/benchmarks/fp_registry.toml)

## Canonical guide

Full workflow, artifact contracts, bucket taxonomy, and decision tree:

**[Manual rule review playbook](../../design/manual-rule-review.md)**

Outcome scoreboard (44/44 PASS as of 2026-06-16, cost-ranked ≤100 sample):

**[Rule TP coverage](../../design/rule-tp-coverage.md)**

## Command cheat sheet

```bash
# Cost-ranked review queue
python3 scripts/rule_tp_census.py --emit-evidence
python3 scripts/rule_tp_census.py --rule SQLCOST012 --sample-cap 100

# Bucket one rule on Spellbook
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST012

# Cost-ranked review packets
python3 scripts/top_findings_review.py --repo spellbook --top 50

# Registry and corpus gates (CI)
python3 scripts/validate_fp_registry.py
python3 scripts/recall_report.py
```

After registry or rule changes, refresh baselines and run `./scripts/ci_local.sh`.

## Related

- [Benchmark tiers](benchmark-tiers.md)
- [Corpus fixtures](corpus-fixtures.md)
- [Classification metrics](../../design/classification-metrics.md)
- [Scripts reference — triage tools](../reference/scripts.md)

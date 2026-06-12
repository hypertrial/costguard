# Corpus and vendored fixtures

## Corpus regression (`tests/fixtures/corpus/`)

Mini dbt projects that assert expected rule IDs per case.

### Add a case

1. Create a directory under `tests/fixtures/corpus/` with `models/` and optional `schema.yml`.
2. Register in [`tests/fixtures/corpus/manifest.toml`](../../../tests/fixtures/corpus/manifest.toml):

```toml
[[case]]
name = "my_case"
path = "my_case"
expect_rules = ["SQLCOST002"]
forbid_rules = ["SQLCOST005"]
```

3. Run `cargo test -p costguard-core --test corpus`.

### Compiled parse fixture

[`spellbook_compiled_parse/`](../../../tests/fixtures/corpus/spellbook_compiled_parse/) asserts zero `sql_parse_compiled_failures` on Trino compiled SQL patterns.

## Vendored real-world snippets (`tests/fixtures/real_world/`)

Minimal offline projects adapted from public repos. See [`tests/fixtures/real_world/README.md`](../../../tests/fixtures/real_world/README.md) for sources, pins, and licenses.

```bash
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/spellbook_snippets
python3 scripts/benchmark_external_repo.py --fixture real_world/manifest_graph
```

Baselines: [`tests/benchmarks/baselines/`](../../../tests/benchmarks/baselines/).

## Calibration loop

When an external benchmark surfaces a finding worth keeping:

1. Triage the diagnostic
2. Extract a minimal snippet into corpus or vendored fixtures
3. Register corpus cases in `manifest.toml`
4. Update baselines with `--update-baseline`
5. Record verdict in [Benchmark calibration](../../design/benchmark-calibration.md)

## Related

- [Benchmark tiers](benchmark-tiers.md)
- [Rule catalog](../rules/index.md)

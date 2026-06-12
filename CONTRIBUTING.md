# Contributing

Use Rust `1.93.1` from `rust-toolchain.toml` and Python 3.11 or newer.

Before submitting a change, run:

```bash
pip install ruff   # required by CI; ci_local.sh warns if missing
./scripts/ci_local.sh
```

Rule behavior changes must include positive and negative corpus fixtures. False-positive fixes should be registered in `tests/benchmarks/fp_registry.toml`. Changes to stable v1 interfaces must follow the [compatibility policy](docs/book/reference/compatibility.md).

External benchmark and release procedures are documented in the [benchmark guide](docs/book/contributing/benchmark-tiers.md) and [release checklist](docs/book/contributing/releasing.md).

Running `mdbook build` writes HTML output to `book/`, `design/`, and `rules/` at the repository root. Those directories are gitignored build artifacts; edit sources under `docs/book`, `docs/design`, and `docs/rules` instead.

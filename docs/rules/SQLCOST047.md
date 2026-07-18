# SQLCOST047: Rocky artifact integrity issue

**Severity:** low

Reports when a configured Rocky project cannot safely use its sealed compile metadata because an artifact is missing, stale, invalid, incompletely mapped, or has unresolved dependencies.

## When it fires

- `rocky.toml` exists but the expected head artifact is unavailable.
- The envelope commit or sealed file hashes do not match the current checkout.
- PR mode cannot verify the supplied base artifact against the resolved comparison commit.
- Rocky dependencies cannot be resolved into the project graph.

## Fix

Compile Rocky with expanded macros and capture the head artifact from a clean committed checkout:

```bash
rocky compile --output json --expand-macros > target/rocky-compile.json
costguard rocky capture --compile target/rocky-compile.json
```

For complete PR comparison, provide a sealed artifact produced for the exact base commit:

```bash
costguard pr \
  --base origin/main \
  --rocky-artifact target/costguard-rocky.json \
  --base-rocky-artifact artifacts/base-costguard-rocky.json
```

## Note

Standard mode reports the integrity warning and avoids unverified compiled SQL. Strict mode or `[rocky].require_artifact_integrity = true` fails analysis until the required artifact verifies.

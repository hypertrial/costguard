# Rocky integration

Costguard analyzes Rocky projects from a sealed compile artifact. Rocky remains responsible for compilation; Costguard never installs, links, or invokes Rocky.

This boundary matters because Rocky's expanded SQL is the only safe input for `.rocky` DSL models and macro-expanded `.sql` models. Sealing the compile result together with every compile input lets Costguard reject stale SQL instead of applying PR gates to output from a different checkout.

## Configure the project

Costguard detects Rocky when `rocky.toml` exists. A complete explicit configuration is:

```toml
[rocky]
config_path = "rocky.toml"
models_dir = "models"
artifact_path = "target/costguard-rocky.json"
base_artifact_path = "artifacts/base-costguard-rocky.json"
max_artifact_bytes = 536870912
require_artifact_integrity = false
```

Do not set `base_artifact_path` unless that file is produced for the exact comparison commit. Unlike the head artifact, Costguard never guesses a base artifact.

## Capture the head artifact

Run Rocky first, with macros expanded, then seal its output:

```bash
mkdir -p target
rocky compile --output json --expand-macros > target/rocky-compile.json
costguard rocky capture \
  --compile target/rocky-compile.json \
  --rocky-config rocky.toml \
  --models-dir models \
  --output target/costguard-rocky.json
```

Use repeatable `--input PATH` arguments for compile inputs outside the standard Rocky config, models, macros, contracts, and groups paths:

```bash
costguard rocky capture \
  --compile target/rocky-compile.json \
  --input defaults.toml \
  --input shared/compile-inputs
```

Capture requires a successful Rocky `compile` payload with expanded SQL for every model. Every sealed input must be a tracked regular file whose bytes match Git `HEAD`; untracked files, dirty inputs, escaping symlinks, ambiguous model sources, and incomplete compile output fail the command. Commit source changes before capture.

## Run a scan

The auto-detected head artifact is enough for a full scan:

```bash
costguard scan
```

An explicit path overrides configuration:

```bash
costguard scan --rocky-artifact target/costguard-rocky.json
```

Costguard verifies the artifact commit and all current file hashes before using compiled SQL. It analyzes `.rocky` models with sealed expanded SQL. Rocky-owned `.sql` uses raw SQL only when it is identical to the expanded SQL; otherwise Costguard analyzes the expansion. Shared SQL and warehouse rules apply, while dbt-only configuration, contract, and manifest rules do not.

Without a usable artifact, standard mode warns, scans Rocky `.sql` as raw framework-neutral SQL, and skips `.rocky` DSL models. Strict mode, or `require_artifact_integrity = true`, fails closed.

## Compare a PR

A complete Rocky finding delta needs artifacts for both the head and the exact base commit:

```bash
costguard pr \
  --base origin/main \
  --rocky-artifact target/costguard-rocky.json \
  --base-rocky-artifact artifacts/base-costguard-rocky.json
```

Two base-artifact workflows are supported:

1. Download the sealed envelope produced by CI for the exact commit Costguard resolves from `--base`.
2. Check out that commit in a separate clean worktree, run Rocky compile and `costguard rocky capture` there, then copy the envelope into the PR checkout.

Costguard verifies the base envelope commit and hashes every sealed input from immutable Git objects. It does not trust files from the current working tree for the base.

If the base artifact is absent or invalid in standard mode, Costguard still reports head Rocky findings but leaves them unclassified and excludes them from `block_only_new` enforcement. Strict mode fails the analysis instead.

Changed Rocky model sources and sidecar TOML files directly affect their models. Changes to global sealed inputs, including `rocky.toml`, macros, contracts, groups, and explicit inputs, conservatively affect every Rocky model. PR output includes dependency descendants and one honest `rocky run --model <name>` recommendation per directly changed model.

## Mixed dbt and Rocky projects

Costguard merges dbt and Rocky metadata into a framework-qualified project graph, so equal model names do not collide. A source path claimed by both frameworks is an error because Costguard cannot safely choose which compiler output governs it.

Rocky's `tags.owner` participates as an explicit owner. Other Rocky tags are normalized as `key=value` for `[owners.tags]` routing. Existing owner precedence remains: explicit model owner, configured tag/path maps, CODEOWNERS, framework group, then default.

Rocky `cost_hint` is not used for enforcement. Rocky costs are mapped only from Costguard's configured catalog, query-history, or observation inputs and pricing.

## Source locations and suppressions

Expanded `.rocky` and macro-transformed `.sql` findings use `source_provenance: "compiled_unmapped"`. JSON retains the model source path and compiled line/column, while GitHub and SARIF emit repository-level results rather than a fabricated line-1 annotation.

Inline source suppressions apply only when raw SQL is analyzed unchanged. Compiled-unmapped findings remain suppressible through finding baselines, policy exceptions, and waivers.

## GitHub Action

Install Costguard, then compile and capture Rocky before the Costguard Action. Download or generate the exact base envelope:

```yaml
- run: |
    export COSTGUARD_INSTALL_DIR="$RUNNER_TEMP/costguard-bin"
    curl -fsSL https://raw.githubusercontent.com/hypertrial/costguard/main/scripts/install.sh | sh -s -- v2.6.0
    rocky compile --output json --expand-macros > target/rocky-compile.json
    "$COSTGUARD_INSTALL_DIR/costguard" rocky capture --compile target/rocky-compile.json
- uses: hypertrial/costguard/.github/actions/costguard@v2.6.0
  with:
    base: origin/main
    rocky-artifact: target/costguard-rocky.json
    base-rocky-artifact: artifacts/base-costguard-rocky.json
    block-only-new: true
```

The Action installs Costguard for its own scan step and forwards artifact paths, but it does not install or execute Rocky. The earlier explicit Costguard install provides the capture binary before the Action starts.

## Related

- [Quick start](quick-start.md)
- [Configuration](../reference/configuration.md)
- [CLI reference](../reference/cli.md)
- [Output formats](../reference/output.md)

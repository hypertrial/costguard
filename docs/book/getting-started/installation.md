# Installation

## One-liner (recommended)

From macOS or Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/hypertrial/costguard/main/scripts/install.sh | sh
```

Pin a release:

```bash
curl -fsSL https://raw.githubusercontent.com/hypertrial/costguard/main/scripts/install.sh | sh -s -- v2.6.0
```

Install to a custom directory:

```bash
COSTGUARD_INSTALL_DIR="$HOME/.local/bin" curl -fsSL .../install.sh | sh
```

The script downloads a release tarball, verifies its SHA256 checksum, and installs the `costguard` binary.

## Build from source (cargo)

Requires a Rust toolchain:

```bash
cargo install --git https://github.com/hypertrial/costguard --tag v2.6.0 costguard-cli
```

## Pinned / airgapped manual install

Select one of `aarch64-apple-darwin`, `x86_64-apple-darwin`, or `x86_64-unknown-linux-gnu`:

```bash
VERSION=v2.6.0
TARGET=aarch64-apple-darwin
curl -LO "https://github.com/hypertrial/costguard/releases/download/${VERSION}/costguard-${TARGET}.tar.gz"
curl -LO "https://github.com/hypertrial/costguard/releases/download/${VERSION}/costguard-${TARGET}.tar.gz.sha256"
shasum -a 256 -c "costguard-${TARGET}.tar.gz.sha256"
tar -xzf "costguard-${TARGET}.tar.gz"
./costguard --version
```

Windows x86-64 uses `costguard-x86_64-pc-windows-msvc.tar.gz` and contains `costguard.exe`. Every release includes consolidated `SHA256SUMS`, native smoke receipts, and signed provenance.

## Verify installation

```bash
costguard --version
costguard rules --format text | head
```

## Next steps

- [Requirements](requirements.md) — what Costguard needs from your dbt project
- [Local scan and explain](local-scan.md) — run `costguard scan` locally
- [Quick start (PR check)](quick-start.md) — add CI with `costguard init` or the GitHub Action

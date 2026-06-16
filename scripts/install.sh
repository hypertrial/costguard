#!/bin/sh
# ponytail: Unix-only installer; Windows uses the documented manual/pwsh path.
set -eu

REPO="${COSTGUARD_REPO:-hypertrial/costguard}"
BIN_NAME="costguard"

version="${COSTGUARD_VERSION:-latest}"
if [ "$#" -ge 1 ] && [ -n "$1" ]; then
  version="$1"
fi

host_target() {
  system="$(uname -s)"
  machine="$(uname -m | tr '[:upper:]' '[:lower:]')"
  case "$system" in
    Linux)
      case "$machine" in
        x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
        *) echo "unsupported host platform: $system-$machine" >&2; exit 1 ;;
      esac
      ;;
    Darwin)
      case "$machine" in
        arm64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) echo "unsupported host platform: $system-$machine" >&2; exit 1 ;;
      esac
      ;;
    *)
      echo "unsupported host platform: $system-$machine" >&2
      exit 1
      ;;
  esac
}

TARGET="$(host_target)"
ASSET="costguard-${TARGET}.tar.gz"
CHECKSUM="${ASSET}.sha256"

if [ -n "${COSTGUARD_RELEASE_BASE_URL:-}" ]; then
  BASE_URL="${COSTGUARD_RELEASE_BASE_URL%/}"
else
  if [ "$version" = "latest" ]; then
    BASE_URL="https://github.com/${REPO}/releases/latest/download"
  else
    BASE_URL="https://github.com/${REPO}/releases/download/${version}"
  fi
fi

install_dir="${COSTGUARD_INSTALL_DIR:-}"
if [ -z "$install_dir" ]; then
  if [ -w /usr/local/bin ] 2>/dev/null; then
    install_dir="/usr/local/bin"
  else
    install_dir="${HOME}/.local/bin"
  fi
fi
mkdir -p "$install_dir"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

download() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    echo "curl or wget is required" >&2
    exit 1
  fi
}

download "${BASE_URL}/${ASSET}" "${tmpdir}/${ASSET}"
download "${BASE_URL}/${CHECKSUM}" "${tmpdir}/${CHECKSUM}"

checksum_line="$(cat "${tmpdir}/${CHECKSUM}")"
expected="$(printf '%s' "$checksum_line" | awk '{print $1}')"
checksum_asset="$(printf '%s' "$checksum_line" | awk '{print $2}')"
if [ "$checksum_asset" != "$ASSET" ]; then
  echo "invalid checksum file for ${ASSET}" >&2
  exit 1
fi

actual=""
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${tmpdir}/${ASSET}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "${tmpdir}/${ASSET}" | awk '{print $1}')"
else
  echo "sha256sum or shasum is required" >&2
  exit 1
fi

if [ "$actual" != "$expected" ]; then
  echo "checksum mismatch for ${ASSET}: expected ${expected}, got ${actual}" >&2
  exit 1
fi

tar -xzf "${tmpdir}/${ASSET}" -C "$tmpdir"
installed="${install_dir}/${BIN_NAME}"
mv "${tmpdir}/${BIN_NAME}" "$installed"
chmod +x "$installed"

echo "installed ${installed}"
"${installed}" --version

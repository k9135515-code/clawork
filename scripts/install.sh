#!/usr/bin/env bash
set -euo pipefail

REPO="${CLAWORK_REPO:-your-org/clawork}"
VERSION="${CLAWORK_VERSION:-latest}"
INSTALL_DIR="${CLAWORK_INSTALL_DIR:-$HOME/.local/bin}"

mkdir -p "$INSTALL_DIR"

if [[ "$REPO" == "your-org/clawork" ]]; then
  echo "Set CLAWORK_REPO=<owner/repo> before running installer." >&2
  exit 1
fi

if [[ "$VERSION" == "latest" ]]; then
  API_URL="https://api.github.com/repos/$REPO/releases/latest"
  TAG=$(curl -fsSL "$API_URL" | sed -n 's/.*"tag_name": "\([^"]*\)".*/\1/p' | head -n1)
else
  TAG="$VERSION"
fi

if [[ -z "${TAG:-}" ]]; then
  echo "Failed to resolve release tag" >&2
  exit 1
fi

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$OS" in
  linux) OS="linux" ;;
  darwin) OS="macos" ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac
case "$ARCH" in
  x86_64) ARCH="x86_64" ;;
  *) echo "Unsupported arch for published artifacts: $ARCH (expected x86_64)" >&2; exit 1 ;;
esac

ASSET="clawork-${OS}-${ARCH}.tar.gz"
URL="https://github.com/$REPO/releases/download/$TAG/$ASSET"
CHECKSUM_URL="${URL}.sha256"

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fL "$URL" -o "$TMP_DIR/$ASSET"
if [[ "${CLAWORK_SKIP_CHECKSUM:-0}" != "1" ]]; then
  curl -fL "$CHECKSUM_URL" -o "$TMP_DIR/$ASSET.sha256"
  EXPECTED_SHA=$(awk '{print $1}' "$TMP_DIR/$ASSET.sha256" | head -n1)
  if [[ -z "${EXPECTED_SHA:-}" ]]; then
    echo "Failed to parse checksum from $CHECKSUM_URL" >&2
    exit 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_SHA=$(sha256sum "$TMP_DIR/$ASSET" | awk '{print $1}')
  else
    ACTUAL_SHA=$(shasum -a 256 "$TMP_DIR/$ASSET" | awk '{print $1}')
  fi
  if [[ "$EXPECTED_SHA" != "$ACTUAL_SHA" ]]; then
    echo "Checksum mismatch for $ASSET" >&2
    echo "expected: $EXPECTED_SHA" >&2
    echo "actual:   $ACTUAL_SHA" >&2
    exit 1
  fi
fi
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"
install -m 0755 "$TMP_DIR/clawork" "$INSTALL_DIR/clawork"

echo "Installed clawork to $INSTALL_DIR/clawork"

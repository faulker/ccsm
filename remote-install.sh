#!/usr/bin/env bash
set -euo pipefail

REPO="faulker/ccsm"
BINARY_NAME="ccsm"
INSTALL_DIR="${HOME}/.local/bin"

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}" in
    Darwin) OS_TARGET="apple-darwin" ;;
    Linux)  OS_TARGET="unknown-linux-gnu" ;;
    *)
        echo "Error: unsupported OS '${OS}'"
        exit 1
        ;;
esac

case "${ARCH}" in
    arm64|aarch64) ARCH_TARGET="aarch64" ;;
    x86_64)        ARCH_TARGET="x86_64" ;;
    *)
        echo "Error: unsupported architecture '${ARCH}'"
        exit 1
        ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"

echo "Detecting latest release..."
LATEST_TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')

if [ -z "$LATEST_TAG" ]; then
    echo "Error: could not determine latest release"
    exit 1
fi

ASSET_NAME="${BINARY_NAME}-${LATEST_TAG}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/${ASSET_NAME}"

echo "Downloading ${BINARY_NAME} ${LATEST_TAG} for ${TARGET}..."

TMPDIR_PATH=$(mktemp -d)
trap 'rm -rf "$TMPDIR_PATH"' EXIT

curl -fSL --progress-bar -o "${TMPDIR_PATH}/${ASSET_NAME}" "$DOWNLOAD_URL"

echo "Extracting..."
tar -xzf "${TMPDIR_PATH}/${ASSET_NAME}" -C "$TMPDIR_PATH"

if [ ! -f "${TMPDIR_PATH}/${BINARY_NAME}" ]; then
    echo "Error: binary not found in archive"
    exit 1
fi

mkdir -p "$INSTALL_DIR"
mv "${TMPDIR_PATH}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

echo "Installed ${BINARY_NAME} ${LATEST_TAG} to ${INSTALL_DIR}/${BINARY_NAME}"

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    echo ""
    echo "Warning: ${INSTALL_DIR} is not in your PATH."
    echo "Add it with:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi

echo "Done! Run '${BINARY_NAME}' to get started."

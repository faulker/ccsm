#!/usr/bin/env bash
set -euo pipefail

BINARY_NAME="ccsm"
INSTALL_DIR="${HOME}/.local/bin"

echo "Building ${BINARY_NAME} in release mode..."
cargo build --release

BINARY_PATH="$(cd "$(dirname "$0")" && pwd)/target/release/${BINARY_NAME}"

if [[ ! -f "$BINARY_PATH" ]]; then
    echo "Error: binary not found at ${BINARY_PATH}"
    exit 1
fi

mkdir -p "$INSTALL_DIR"

ln -sf "$BINARY_PATH" "${INSTALL_DIR}/${BINARY_NAME}"
echo "Symlinked ${BINARY_PATH} -> ${INSTALL_DIR}/${BINARY_NAME}"

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    echo ""
    echo "Warning: ${INSTALL_DIR} is not in your PATH."
    echo "Add it with:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi

echo "Done! You can now run '${BINARY_NAME}'."

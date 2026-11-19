#!/usr/bin/env bash
# RustCode Packaging Script - Creates a distribution bundle
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DIST_DIR="${REPO_ROOT}/dist"
TARGET_NAME="rustcode"
VERSION="${1:-0.1.0}"
ARCH="$(uname -m)"
RAW_OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
BIN_EXT=""
case "${RAW_OS}" in
    mingw*|msys*|cygwin*)
        OS="windows"
        BIN_EXT=".exe"
        ;;
    *)
        OS="${RAW_OS}"
        ;;
esac
PACKAGE_NAME="${TARGET_NAME}-${VERSION}-${OS}-${ARCH}"
BUNDLE_DIR="${DIST_DIR}/${PACKAGE_NAME}"
BIN_NAME="${TARGET_NAME}${BIN_EXT}"

echo "==> Packaging RustCode v${VERSION} for ${OS}-${ARCH}..."

rm -rf "${BUNDLE_DIR}" "${DIST_DIR}/${PACKAGE_NAME}.tar.gz"
mkdir -p "${BUNDLE_DIR}/bin"
mkdir -p "${BUNDLE_DIR}/resources"
mkdir -p "${BUNDLE_DIR}/ext-host-js"
if [ "${OS}" = "linux" ]; then
    mkdir -p "${BUNDLE_DIR}/share/applications"
fi

# Copy binary if prebuilt in release (honoring CARGO_TARGET_DIR), or provide build instruction
TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
if [ -f "${TARGET_DIR}/release/${BIN_NAME}" ]; then
    echo "==> Bundling release binary..."
    cp "${TARGET_DIR}/release/${BIN_NAME}" "${BUNDLE_DIR}/bin/"
elif [ -f "${REPO_ROOT}/target/release/${BIN_NAME}" ]; then
    echo "==> Bundling release binary..."
    cp "${REPO_ROOT}/target/release/${BIN_NAME}" "${BUNDLE_DIR}/bin/"
elif [ -f "${HOME}/.cargo_target/release/${BIN_NAME}" ]; then
    echo "==> Bundling release binary from custom target..."
    cp "${HOME}/.cargo_target/release/${BIN_NAME}" "${BUNDLE_DIR}/bin/"
else
    echo "==> Release binary not found locally. To compile: cargo build --release --bin rustcode"
    touch "${BUNDLE_DIR}/bin/.build_required"
fi

# Bundle resources
echo "==> Bundling resources and extension host shims..."
if [ -d "${REPO_ROOT}/resources" ]; then
    cp -r "${REPO_ROOT}/resources/"* "${BUNDLE_DIR}/resources/"
fi

if [ -d "${REPO_ROOT}/ext-host-js" ]; then
    cp -r "${REPO_ROOT}/ext-host-js/"* "${BUNDLE_DIR}/ext-host-js/"
fi

# Bundle desktop entry (Linux only)
if [ "${OS}" = "linux" ] && [ -f "${REPO_ROOT}/resources/rustcode.desktop" ]; then
    cp "${REPO_ROOT}/resources/rustcode.desktop" "${BUNDLE_DIR}/share/applications/"
fi

# Create archive
cd "${DIST_DIR}"
tar -czf "${PACKAGE_NAME}.tar.gz" "${PACKAGE_NAME}"
echo "==> Distribution archive created at: ${DIST_DIR}/${PACKAGE_NAME}.tar.gz"

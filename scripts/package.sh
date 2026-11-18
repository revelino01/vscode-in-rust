#!/usr/bin/env bash
# RustCode Packaging Script - Creates a distribution bundle
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DIST_DIR="${REPO_ROOT}/dist"
TARGET_NAME="rustcode"
VERSION="${1:-0.1.0}"
ARCH="$(uname -m)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
PACKAGE_NAME="${TARGET_NAME}-${VERSION}-${OS}-${ARCH}"
BUNDLE_DIR="${DIST_DIR}/${PACKAGE_NAME}"

echo "==> Packaging RustCode v${VERSION} for ${OS}-${ARCH}..."

rm -rf "${DIST_DIR}"
mkdir -p "${BUNDLE_DIR}/bin"
mkdir -p "${BUNDLE_DIR}/resources"
mkdir -p "${BUNDLE_DIR}/ext-host-js"
mkdir -p "${BUNDLE_DIR}/share/applications"

# Copy binary if prebuilt in release, or provide build instruction
if [ -f "${REPO_ROOT}/target/release/rustcode" ]; then
    echo "==> Bundling release binary..."
    cp "${REPO_ROOT}/target/release/rustcode" "${BUNDLE_DIR}/bin/"
elif [ -f "${HOME}/.cargo_target/release/rustcode" ]; then
    echo "==> Bundling release binary from custom target..."
    cp "${HOME}/.cargo_target/release/rustcode" "${BUNDLE_DIR}/bin/"
else
    echo "==> Release binary not found locally. To compile: cargo build --release -p rustcode-app"
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

# Bundle desktop entry
if [ -f "${REPO_ROOT}/resources/rustcode.desktop" ]; then
    cp "${REPO_ROOT}/resources/rustcode.desktop" "${BUNDLE_DIR}/share/applications/"
fi

# Create archive
cd "${DIST_DIR}"
tar -czf "${PACKAGE_NAME}.tar.gz" "${PACKAGE_NAME}"
echo "==> Distribution archive created at: ${DIST_DIR}/${PACKAGE_NAME}.tar.gz"

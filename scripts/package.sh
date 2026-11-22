#!/usr/bin/env bash
# RustCode Packaging Script - Creates a distribution bundle
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DIST_DIR="${REPO_ROOT}/dist"
TARGET_NAME="rustcode"
VERSION="0.1.0"
BUILD=false
DESKTOP=false
JOBS=2
POSITIONAL_VERSION=""

print_usage() {
    cat <<'EOF'
Usage: package.sh [VERSION] [options]

Options:
  -v, --version <ver>   Package version (default: 0.1.0)
  -b, --build           Build release binary before packaging
  -d, --desktop         Enable Cargo --features desktop when building
  -j, --jobs <n>        Parallel Cargo jobs (default: 2)
  -h, --help            Show this help message
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            print_usage
            exit 0
            ;;
        -b|--build)
            BUILD=true
            shift
            ;;
        -d|--desktop)
            DESKTOP=true
            shift
            ;;
        -v|--version)
            if [ "$#" -lt 2 ]; then
                echo "Error: $1 requires a version argument" >&2
                exit 2
            fi
            VERSION="$2"
            shift 2
            ;;
        --version=*)
            VERSION="${1#*=}"
            shift
            ;;
        -j|--jobs)
            if [ "$#" -lt 2 ]; then
                echo "Error: $1 requires a numeric argument" >&2
                exit 2
            fi
            JOBS="$2"
            shift 2
            ;;
        --jobs=*)
            JOBS="${1#*=}"
            shift
            ;;
        -*)
            echo "Error: Unknown option '$1'" >&2
            print_usage >&2
            exit 2
            ;;
        *)
            if [ -z "${POSITIONAL_VERSION}" ]; then
                POSITIONAL_VERSION="$1"
                VERSION="$1"
                shift
            else
                echo "Error: Unexpected positional argument '$1'" >&2
                print_usage >&2
                exit 2
            fi
            ;;
    esac
done

if ! [[ "${JOBS}" =~ ^[1-9][0-9]*$ ]]; then
    echo "Error: --jobs must be a positive integer (got '${JOBS}')" >&2
    exit 2
fi

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

CARGO_BUILD_ARGS=(build --release -j "${JOBS}" --bin "${TARGET_NAME}")
if [ "${DESKTOP}" = "true" ]; then
    CARGO_BUILD_ARGS+=(--features desktop)
fi

if [ "${BUILD}" = "true" ]; then
    echo "==> Building release binary (cargo ${CARGO_BUILD_ARGS[*]})..."
    (
        cd "${REPO_ROOT}"
        cargo "${CARGO_BUILD_ARGS[@]}"
    )
fi

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
elif [ "${BUILD}" = "true" ]; then
    echo "Error: Release binary '${BIN_NAME}' was not found after build." >&2
    exit 1
else
    echo "==> Release binary not found locally. To compile: cargo ${CARGO_BUILD_ARGS[*]}"
    touch "${BUNDLE_DIR}/bin/.build_required"
fi

if [ -f "${BUNDLE_DIR}/bin/${BIN_NAME}" ] && [ -z "${BIN_EXT}" ]; then
    chmod +x "${BUNDLE_DIR}/bin/${BIN_NAME}"
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

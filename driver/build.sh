#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DRIVER_NAME="PureaDriver"
BUNDLE_NAME="${DRIVER_NAME}.driver"
BUILD_DIR="${SCRIPT_DIR}/build"

echo "Building ${BUNDLE_NAME}..."

rm -rf "${BUILD_DIR}/${BUNDLE_NAME}"
mkdir -p "${BUILD_DIR}/${BUNDLE_NAME}/Contents/MacOS"

# Compile for both arm64 and x86_64
clang -bundle \
    -arch arm64 -arch x86_64 \
    -mmacosx-version-min=13.0 \
    -framework CoreAudio \
    -framework CoreFoundation \
    -o "${BUILD_DIR}/${BUNDLE_NAME}/Contents/MacOS/${DRIVER_NAME}" \
    "${SCRIPT_DIR}/PureaDriver.c"

# Copy Info.plist
cp "${SCRIPT_DIR}/Info.plist" "${BUILD_DIR}/${BUNDLE_NAME}/Contents/Info.plist"

# Sign the bundle with ad-hoc signature (required for CoreAudio to load it)
codesign --force --sign - "${BUILD_DIR}/${BUNDLE_NAME}"

echo "Built and signed: ${BUILD_DIR}/${BUNDLE_NAME}"
echo ""
echo "To install (requires admin):"
echo "  sudo cp -R ${BUILD_DIR}/${BUNDLE_NAME} /Library/Audio/Plug-Ins/HAL/"
echo "  sudo chown -R root:wheel /Library/Audio/Plug-Ins/HAL/${BUNDLE_NAME}"
echo "  sudo killall coreaudiod"

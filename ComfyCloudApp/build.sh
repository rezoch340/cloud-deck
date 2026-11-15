#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
COLAB_DIR="$(dirname "$ROOT_DIR")"
BUILD_TMP="${COMFY_BUILD_DIR:-/Volumes/VoltDrive/AI/tmp/cloud-deck/swift-build}"
mkdir -p "$BUILD_TMP/tmp" "$BUILD_TMP/module-cache"
export TMPDIR="$BUILD_TMP/tmp"
export CLANG_MODULE_CACHE_PATH="$BUILD_TMP/module-cache"
export SWIFT_MODULECACHE_PATH="$BUILD_TMP/module-cache"
APP_BUNDLE="$COLAB_DIR/Comfy Cloud.app"
CONTENTS="$APP_BUNDLE/Contents"

mkdir -p "$BUILD_TMP" "$CONTENTS/MacOS" "$CONTENTS/Resources"

xcrun swiftc \
  -O \
  -target arm64-apple-macosx14.0 \
  -parse-as-library \
  -module-cache-path "$BUILD_TMP/module-cache" \
  -framework AppKit \
  -framework SwiftUI \
  "$ROOT_DIR/Sources/Models.swift" \
  "$ROOT_DIR/Sources/CloudController.swift" \
  "$ROOT_DIR/Sources/ContentView.swift" \
  "$ROOT_DIR/Sources/ComfyCloudApp.swift" \
  -o "$BUILD_TMP/ComfyCloud"

cp "$BUILD_TMP/ComfyCloud" "$CONTENTS/MacOS/ComfyCloud"
cp "$ROOT_DIR/Info.plist" "$CONTENTS/Info.plist"
codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null 2>&1 || true

echo "Built: $APP_BUNDLE"

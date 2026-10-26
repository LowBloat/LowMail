#!/usr/bin/env bash
# Generates the Android project (src-tauri/gen/android) with the app icons and
# LowMail's own MainActivity. Needs the Android SDK and NDK (NDK_HOME).
set -euo pipefail
bun run tauri android init --ci
bun run tauri icon src-tauri/icons/icon.png
activity=$(find src-tauri/gen/android/app/src/main -name MainActivity.kt | head -n 1)
[ -n "$activity" ] || { echo "MainActivity.kt not generated"; exit 1; }
cp packaging/android/MainActivity.kt "$activity"
grep -E "targetSdk|minSdk" src-tauri/gen/android/app/build.gradle.kts

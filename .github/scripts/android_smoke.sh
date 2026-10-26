#!/usr/bin/env bash
# Installs the debug APK on the running emulator, opens LowMail and drives it
# through the WebView's DevTools socket (android_smoke.mjs). Screenshots and
# the log go to the folder given as the first argument.
set -euo pipefail
out="$1"
mkdir -p "$out"
package=dev.lowbloat.lowmail

apk=$(find src-tauri/gen/android/app/build/outputs/apk -name '*debug*.apk' | head -n 1)
echo "Installing $apk"
adb install -r "$apk"
adb logcat -c || true
adb shell monkey -p "$package" -c android.intent.category.LAUNCHER 1

pid=""
for _ in $(seq 1 60); do
	pid=$(adb shell pidof "$package" | tr -d '\r' || true)
	[ -n "$pid" ] && adb shell cat /proc/net/unix | grep -q "webview_devtools_remote_$pid" && break
	sleep 2
done
[ -n "$pid" ] || { echo "LowMail did not start"; adb logcat -d > "$out/logcat.txt"; exit 1; }
adb forward tcp:9222 "localabstract:webview_devtools_remote_$pid"
sleep 5
adb exec-out screencap -p > "$out/1-start.png"

status=0
bun .github/scripts/android_smoke.mjs "$out" || status=$?

adb exec-out screencap -p > "$out/3-final.png"
adb logcat -d > "$out/logcat.txt"
exit $status

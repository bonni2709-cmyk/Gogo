#!/usr/bin/env bash
set -euo pipefail
# Always save logcat, including NativeActivity / Rust aborts during startup.
adb logcat -c
finish() { adb logcat -d > emulator-logcat.txt || true; }
trap finish EXIT
(cd android && gradle --no-daemon -PlumenAbi=x86_64 :app:connectedDebugAndroidTest) 2>&1 | tee instrumentation.log

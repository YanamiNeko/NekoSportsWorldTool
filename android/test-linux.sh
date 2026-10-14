#!/usr/bin/env bash
# Run instrumentation only on the isolated emulator created by CI.
set -euo pipefail
: "${ANDROID_HOME:?Android SDK is required}"
: "${PACKAGE_DIR:?The isolated test APK directory is required}"
: "${KEYSTORE:?The matching independent test keystore is required}"
serial="${ANDROID_SERIAL:-}"
if [[ -z "$serial" ]]; then
    mapfile -t devices < <(adb devices | awk '$1 ~ /^emulator-[0-9]+$/ && $2 == "device" {print $1}')
    if [[ ${#devices[@]} != 1 ]]; then echo 'Expected exactly one connected emulator'; exit 1; fi
    serial="${devices[0]}"
fi
if [[ ! "$serial" =~ ^emulator-[0-9]+$ ]]; then echo 'Refusing to operate on a physical device'; exit 1; fi
adb_cmd=(adb -s "$serial")
BT="$ANDROID_HOME/build-tools/${BUILD_TOOLS:-36.0.0}"
ANDROID_JAR="$ANDROID_HOME/platforms/${PLATFORM:-android-36}/android.jar"
diagnostics="$PACKAGE_DIR/diagnostics"
out="$PACKAGE_DIR/instrumentation"
mkdir -p "$diagnostics" "$out/classes" "$out/dex"
capture_diagnostics() {
    "${adb_cmd[@]}" logcat -d -v threadtime > "$diagnostics/logcat.txt" 2>&1 || true
    "${adb_cmd[@]}" shell dumpsys activity activities > "$diagnostics/activities.txt" 2>&1 || true
}
trap capture_diagnostics EXIT
javac -encoding UTF-8 -source 8 -target 8 -Xlint:-options \
    -bootclasspath "$ANDROID_JAR:$BT/core-lambda-stubs.jar" -classpath "$PACKAGE_DIR/classes.jar" \
    -d "$out/classes" android/tests/SmokeInstrumentation.java
jar cf "$out/tests.jar" -C "$out/classes" .
"$BT/d8" --lib "$ANDROID_JAR" --classpath "$PACKAGE_DIR/classes.jar" \
    --min-api 26 --output "$out/dex" "$out/tests.jar"
"$BT/aapt2" link -o "$out/unsigned.apk" -I "$ANDROID_JAR" --manifest android/tests/AndroidManifest.xml
(cd "$out/dex" && zip -q -X ../unsigned.apk classes.dex)
"$BT/apksigner" sign --ks "$KEYSTORE" --ks-key-alias androiddebugkey \
    --ks-pass pass:android --key-pass pass:android --out "$out/tests.apk" "$out/unsigned.apk"
"${adb_cmd[@]}" install -r "$PACKAGE_DIR/app.apk"
"${adb_cmd[@]}" install -r "$out/tests.apk"
"${adb_cmd[@]}" shell input keyevent 224
"${adb_cmd[@]}" shell wm dismiss-keyguard
"${adb_cmd[@]}" logcat -c
"${adb_cmd[@]}" shell am start -W -n org.nekosportsworld.tool/.MainActivity > "$diagnostics/launch.txt"
pid="$("${adb_cmd[@]}" shell pidof org.nekosportsworld.tool | tr -d '\r')"
test -n "$pid"
for ((second = 0; second < 70; second++)); do
    sleep 1
    current="$("${adb_cmd[@]}" shell pidof org.nekosportsworld.tool | tr -d '\r')"
    if [[ "$current" != "$pid" ]]; then echo 'App died or restarted during the 70-second startup check'; exit 1; fi
done
"${adb_cmd[@]}" shell dumpsys activity activities > "$diagnostics/startup-activities.txt"
grep -E '(mResumedActivity:|topResumedActivity=).*org\.nekosportsworld\.tool/(\.MainActivity|org\.nekosportsworld\.tool\.MainActivity)' "$diagnostics/startup-activities.txt"
"${adb_cmd[@]}" logcat -d --pid="$pid" -v threadtime > "$diagnostics/startup-logcat.txt"
if grep -E 'FATAL EXCEPTION|panicked at|Fatal signal [0-9]+' "$diagnostics/startup-logcat.txt"; then
    echo 'Fatal startup error detected'; exit 1
fi
"${adb_cmd[@]}" shell am instrument -w \
    org.nekosportsworld.tool.tests/org.nekosportsworld.tool.tests.SmokeInstrumentation | tee "$diagnostics/instrumentation.txt"
grep -E 'PASS: [0-9]+ Android integration checks' "$diagnostics/instrumentation.txt"
"${adb_cmd[@]}" shell am instrument -w -e verifyStoredBrand true \
    org.nekosportsworld.tool.tests/org.nekosportsworld.tool.tests.SmokeInstrumentation | tee "$diagnostics/persistence.txt"
grep -E 'PASS: [0-9]+ Android persistence restart checks' "$diagnostics/persistence.txt"
echo 'PASS: release startup stayed alive for 70 seconds; Activity and process-restart checks passed'

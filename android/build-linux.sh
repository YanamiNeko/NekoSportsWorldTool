#!/usr/bin/env bash
# Build a release-mode emulator APK with an isolated test key.
set -euo pipefail
: "${ANDROID_HOME:?Android SDK is required}"
: "${PACKAGE_DIR:?An isolated packaging directory is required}"
: "${KEYSTORE:?An independent test keystore is required}"
ABI="${ABI:-x86_64}"
TARGET="${TARGET:-x86_64-linux-android}"
NDK_VERSION="${NDK_VERSION:-29.0.14206865}"
BUILD_TOOLS="${BUILD_TOOLS:-36.0.0}"
PLATFORM="${PLATFORM:-android-36}"
BT="$ANDROID_HOME/build-tools/$BUILD_TOOLS"
ANDROID_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"
NDK_BIN="$ANDROID_HOME/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"

target_key="${TARGET//-/_}"
export "CARGO_TARGET_${target_key^^}_LINKER=$NDK_BIN/${TARGET}26-clang"
export "CC_${target_key}=$NDK_BIN/${TARGET}26-clang"
export "AR_${target_key}=$NDK_BIN/llvm-ar"
export "CARGO_TARGET_${target_key^^}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
cargo build --locked --release --target "$TARGET" --features android --lib
so="target/$TARGET/release/libnekosportsworldtool.so"
aligns="$("$NDK_BIN/llvm-readelf" -lW "$so" | awk '/^ *LOAD /{print $NF}')"
test -n "$aligns"
for alignment in $aligns; do
    if (( alignment < 16384 )); then echo "ELF LOAD segment is not 16KB aligned"; exit 1; fi
done

mkdir -p "$PACKAGE_DIR/classes" "$PACKAGE_DIR/dex" "$PACKAGE_DIR/staging/lib/$ABI"
cp -r android/java android/res android/AndroidManifest.xml "$PACKAGE_DIR/"
sed -i -E 's/android:versionCode="[0-9]+"/android:versionCode="1"/; s/android:versionName="[^"]*"/android:versionName="0.0.0-smoke"/' "$PACKAGE_DIR/AndroidManifest.xml"
find "$PACKAGE_DIR/java" -name '*.java' > "$PACKAGE_DIR/sources.txt"
javac -encoding UTF-8 -source 8 -target 8 -Xlint:-options \
    -bootclasspath "$ANDROID_JAR:$BT/core-lambda-stubs.jar" \
    -d "$PACKAGE_DIR/classes" @"$PACKAGE_DIR/sources.txt"
jar cf "$PACKAGE_DIR/classes.jar" -C "$PACKAGE_DIR/classes" .
"$BT/d8" --lib "$ANDROID_JAR" --min-api 26 --output "$PACKAGE_DIR/dex" "$PACKAGE_DIR/classes.jar"
"$BT/aapt2" compile --dir "$PACKAGE_DIR/res" -o "$PACKAGE_DIR/resources.zip"
"$BT/aapt2" link -o "$PACKAGE_DIR/unsigned.apk" -I "$ANDROID_JAR" \
    --manifest "$PACKAGE_DIR/AndroidManifest.xml" "$PACKAGE_DIR/resources.zip"
cp "$so" "$PACKAGE_DIR/staging/lib/$ABI/libnekosportsworldtool.so"
cp "$PACKAGE_DIR/dex/classes.dex" "$PACKAGE_DIR/staging/classes.dex"
(cd "$PACKAGE_DIR/staging" && zip -q -X ../unsigned.apk classes.dex "lib/$ABI/libnekosportsworldtool.so")
"$BT/zipalign" -f -P 16 4 "$PACKAGE_DIR/unsigned.apk" "$PACKAGE_DIR/aligned.apk"
"$BT/apksigner" sign --ks "$KEYSTORE" --ks-key-alias androiddebugkey \
    --ks-pass pass:android --key-pass pass:android --out "$PACKAGE_DIR/app.apk" "$PACKAGE_DIR/aligned.apk"
"$BT/apksigner" verify --verbose "$PACKAGE_DIR/app.apk"
"$BT/zipalign" -c -P 16 4 "$PACKAGE_DIR/app.apk"

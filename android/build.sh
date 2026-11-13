#!/usr/bin/env bash
# Build the StreamPipe Android APK without Gradle: core (cargo-ndk),
# resources (aapt2), Java (javac + d8), then align and sign.
#
#   android/build.sh            # arm64-v8a
#   ABIS="arm64-v8a x86_64" android/build.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
# The JDK and SDK tools are Windows programs: give them C:/... paths, not /c/...
if command -v cygpath >/dev/null 2>&1; then
  HERE="$(cygpath -m "$HERE")"
  ROOT="$(cygpath -m "$ROOT")"
fi
SDK="${ANDROID_SDK_ROOT:-C:/android}"
BT="$SDK/build-tools/34.0.0"
PLATFORM="$SDK/platforms/android-34/android.jar"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$SDK/ndk/26.1.10909125}"
JAVA_BIN="${JAVA_HOME:+$JAVA_HOME/bin/}"
ABIS="${ABIS:-arm64-v8a}"
VERSION_NAME="${VERSION_NAME:-0.3.0}"
VERSION_CODE="${VERSION_CODE:-3}"
MIN_SDK=26
OUT="$HERE/build"
APK="$ROOT/dist/StreamPipe-$VERSION_NAME-android.apk"

rm -rf "$OUT"
mkdir -p "$OUT/assets" "$OUT/gen" "$OUT/classes" "$OUT/dex" "$OUT/apk" "$ROOT/dist"

echo "[1/6] assets"
cp "$HERE/app/assets/player-inject.js" "$HERE/app/assets/player.html" "$OUT/assets/"

echo "[2/6] core ($ABIS)"
for abi in $ABIS; do
  (cd "$ROOT" && cargo ndk -t "$abi" --platform $MIN_SDK -o "$OUT/apk/lib" build -p streampipe-android --release)
done

# The yt-parser library resolves videos to stream URLs (build it with
# third_party/yt-parser-src/build-android.sh).
for abi in $ABIS; do
  cp "$ROOT/third_party/yt-parser-src/build/android/$abi/libytp.so" "$OUT/apk/lib/$abi/"
done

echo "[3/6] resources"
"$BT/aapt2" compile --dir "$HERE/app/res" -o "$OUT/res.zip"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$PLATFORM" --manifest "$HERE/app/AndroidManifest.xml" \
  -A "$OUT/assets" --java "$OUT/gen" --min-sdk-version $MIN_SDK --target-sdk-version 34 \
  --version-code "$VERSION_CODE" --version-name "$VERSION_NAME" "$OUT/res.zip"

echo "[4/6] java"
# Media3 (ExoPlayer) and what it needs, from third_party/media3/jars.
LIBS="$(ls "$ROOT"/third_party/media3/jars/*.jar | while read -r j; do cygpath -m "$j"; done | paste -sd ';')"
find "$HERE/app/src" "$OUT/gen" -name '*.java' > "$OUT/sources.txt"
"${JAVA_BIN}javac" -nowarn -source 8 -target 8 -encoding UTF-8 -bootclasspath "$PLATFORM" -classpath "$PLATFORM;$BT/core-lambda-stubs.jar;$LIBS" \
  -d "$OUT/classes" @"$OUT/sources.txt" > "$OUT/javac.log" 2>&1 || { cat "$OUT/javac.log"; exit 1; }
find "$OUT/classes" -name '*.class' > "$OUT/classes.txt"
"${JAVA_BIN}java" -cp "$BT/lib/d8.jar" com.android.tools.r8.D8 --release --lib "$PLATFORM" --min-api $MIN_SDK \
  --output "$OUT/dex" @"$OUT/classes.txt" "$ROOT"/third_party/media3/jars/*.jar

echo "[5/6] package"
cp "$OUT"/dex/classes*.dex "$OUT/apk/"
cp "$OUT/base.apk" "$OUT/unsigned.apk"
python - "$OUT/unsigned.apk" "$OUT/apk" <<'PY'
import os, sys, zipfile
apk, root = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(apk, "a", zipfile.ZIP_DEFLATED) as z:
    for d, _, files in os.walk(root):
        for f in files:
            p = os.path.join(d, f)
            z.write(p, os.path.relpath(p, root).replace(os.sep, "/"))
PY

echo "[6/6] align and sign"
KS="$HERE/debug.keystore"
if [ ! -f "$KS" ]; then
  "${JAVA_BIN}keytool" -genkeypair -keystore "$KS" -storepass android -keypass android -alias streampipe \
    -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=StreamPipe test build" >/dev/null
fi
"$BT/zipalign" -f 4 "$OUT/unsigned.apk" "$OUT/aligned.apk"
"${JAVA_BIN}java" -jar "$BT/lib/apksigner.jar" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android \
  --ks-key-alias streampipe --out "$APK" "$OUT/aligned.apk"
rm -f "$APK.idsig"
ls -la "$APK"

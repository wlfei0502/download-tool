#!/usr/bin/env bash
# Build a macOS .app + .dmg for local install testing.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="${ROOT}/dist/macos"
APP_NAME="Download Tool"
APP="${DIST}/${APP_NAME}.app"
CONTENTS="${APP}/Contents"
MACOS="${CONTENTS}/MacOS"
BIN_NAME="download-tool"
DMG="${DIST}/download-tool.dmg"

mkdir -p "${DIST}"
rm -rf "${APP}" "${DMG}"

echo "==> release build"
cd "${ROOT}"
# Avoid sandbox/redirect target dirs when packaging for the user machine.
unset CARGO_TARGET_DIR
cargo build --release -p download-app

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
EXE="${TARGET_DIR}/release/${BIN_NAME}"
if [[ ! -x "${EXE}" ]]; then
  echo "release binary not found at ${EXE}" >&2
  exit 1
fi
echo "using binary: ${EXE}"

ARIA2C="$(command -v aria2c || true)"
if [[ -z "${ARIA2C}" ]]; then
  echo "aria2c not found on PATH; install with: brew install aria2" >&2
  exit 1
fi

mkdir -p "${MACOS}"
cp "${EXE}" "${MACOS}/${BIN_NAME}.bin"
cp "${ARIA2C}" "${MACOS}/aria2c"
chmod +x "${MACOS}/${BIN_NAME}.bin" "${MACOS}/aria2c"

if [[ -f /etc/ssl/cert.pem ]]; then
  cp /etc/ssl/cert.pem "${MACOS}/cert.pem"
fi

# Copy Homebrew-linked dylibs next to aria2c and rewrite paths.
copy_deps() {
  local binary="$1"
  local libs
  libs="$(otool -L "${binary}" | awk '/^\t(\/opt\/homebrew|\/usr\/local)\// {print $1}')"
  local lib
  for lib in ${libs}; do
    local base
    base="$(basename "${lib}")"
    if [[ ! -f "${MACOS}/${base}" ]]; then
      cp "${lib}" "${MACOS}/${base}"
      chmod +w "${MACOS}/${base}"
      install_name_tool -id "@executable_path/${base}" "${MACOS}/${base}" 2>/dev/null || true
      copy_deps "${MACOS}/${base}"
    fi
    install_name_tool -change "${lib}" "@executable_path/${base}" "${binary}" 2>/dev/null || true
  done
}
copy_deps "${MACOS}/aria2c"

# Launcher: prefer bundled aria2c, else Homebrew.
cat > "${MACOS}/${BIN_NAME}" <<'LAUNCH'
#!/bin/bash
DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ -x "${DIR}/aria2c" ]]; then
  export DOWNLOAD_TOOL_ARIA2C="${DIR}/aria2c"
fi
if [[ -z "${DOWNLOAD_TOOL_ARIA2C:-}" && -x /opt/homebrew/bin/aria2c ]]; then
  export DOWNLOAD_TOOL_ARIA2C="/opt/homebrew/bin/aria2c"
elif [[ -z "${DOWNLOAD_TOOL_ARIA2C:-}" && -x /usr/local/bin/aria2c ]]; then
  export DOWNLOAD_TOOL_ARIA2C="/usr/local/bin/aria2c"
fi
if [[ -f "${DIR}/cert.pem" ]]; then
  export DOWNLOAD_TOOL_CA_FILE="${DIR}/cert.pem"
fi
exec "${DIR}/download-tool.bin" "$@"
LAUNCH
chmod +x "${MACOS}/${BIN_NAME}"

cat > "${CONTENTS}/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>zh_CN</string>
  <key>CFBundleExecutable</key>
  <string>download-tool</string>
  <key>CFBundleIdentifier</key>
  <string>com.download-tool.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Download Tool</string>
  <key>CFBundleDisplayName</key>
  <string>Download Tool</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>
      <string>Download Tool</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>dltool</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
PLIST

echo "APPL????" > "${CONTENTS}/PkgInfo"

if command -v codesign >/dev/null; then
  codesign --force --deep -s - "${APP}" >/dev/null 2>&1 || true
fi

echo "==> create dmg (with Applications drop target)"
STAGE="${DIST}/dmg-stage"
rm -rf "${STAGE}"
mkdir -p "${STAGE}"
cp -R "${APP}" "${STAGE}/"
ln -s /Applications "${STAGE}/Applications"
cat > "${STAGE}/README.txt" <<EOF
Install:
1. Drag "${APP_NAME}" onto the Applications folder
2. Open it from Launchpad or Applications
3. If macOS blocks an unidentified developer: System Settings → Privacy & Security → Open Anyway

After install, large-file downloads from the website can use the local API or dltool://.
EOF

# Staging already has: app + Applications symlink + README.txt
rm -f "${DMG}"
hdiutil create -volname "${APP_NAME}" -srcfolder "${STAGE}" -ov -format UDZO -imagekey zlib-level=9 "${DMG}" >/dev/null
rm -rf "${STAGE}"
rm -f "${DIST}/.Download-Tool-rw.dmg"

LS="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "${LS}" ]]; then
  "${LS}" -f "${APP}" >/dev/null 2>&1 || true
fi

echo
echo "App: ${APP}"
echo "DMG: ${DMG}"
echo
echo "Open the disk image:"
echo "  open \"${DMG}\""
echo "Drag \"${APP_NAME}\" to Applications to install (optional)."

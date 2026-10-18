#!/usr/bin/env bash
# Build a macOS .app + .dmg for local install testing.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="${ROOT}/dist/macos"
APP_NAME="下载器"
APP="${DIST}/${APP_NAME}.app"
CONTENTS="${APP}/Contents"
MACOS="${CONTENTS}/MacOS"
BIN_NAME="download-tool"
DMG="${DIST}/${APP_NAME}.dmg"

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
  <string>下载器</string>
  <key>CFBundleDisplayName</key>
  <string>下载器</string>
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
      <string>下载器</string>
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
cat > "${STAGE}/安装说明.txt" <<EOF
安装步骤：
1. 将「${APP_NAME}」拖到旁边的「Applications」文件夹
2. 打开「启动台」或「应用程序」，启动「${APP_NAME}」
3. 若提示无法验证开发者：系统设置 → 隐私与安全性 → 仍要打开

安装后，网站大文件下载可通过本机接口或 dltool:// 唤起下载器。
EOF

# Staging already has: app + Applications symlink + 安装说明.txt
rm -f "${DMG}"
hdiutil create -volname "${APP_NAME}" -srcfolder "${STAGE}" -ov -format UDZO -imagekey zlib-level=9 "${DMG}" >/dev/null
rm -rf "${STAGE}"
rm -f "${DIST}/.${APP_NAME}-rw.dmg"

LS="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "${LS}" ]]; then
  "${LS}" -f "${APP}" >/dev/null 2>&1 || true
fi

echo
echo "App: ${APP}"
echo "DMG: ${DMG}"
echo
echo "打开安装盘："
echo "  open \"${DMG}\""
echo "把「${APP_NAME}」拖到「Applications」即可安装。"

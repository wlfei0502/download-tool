#!/usr/bin/env bash
# Cross-notes: prefer package-windows-portable.ps1 on a real Windows machine.
# This script only documents the layout expected for a portable build.
set -euo pipefail
echo "Windows portable packaging should be run on Windows:"
echo "  powershell -ExecutionPolicy Bypass -File scripts/package-windows-portable.ps1"
echo
echo "Expected layout (no installer):"
echo "  download-tool/"
echo "    download-tool.exe"
echo "    aria2c.exe"
echo "    cacert.pem   (or curl-ca-bundle.crt)"
echo "    使用说明.txt"
echo
echo "User: unzip anywhere → double-click download-tool.exe"
exit 0

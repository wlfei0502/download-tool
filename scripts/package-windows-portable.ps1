# Build a portable Windows folder + zip. No installer.
# Run on Windows (PowerShell), with Rust and aria2c available.
# Output: dist/windows/download-tool/  and  dist/windows/download-tool-portable.zip

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

$Dist = Join-Path $Root "dist\windows"
$AppName = "download-tool"
$Out = Join-Path $Dist $AppName
$Zip = Join-Path $Dist "$AppName-portable.zip"

New-Item -ItemType Directory -Force -Path $Dist | Out-Null
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
if (Test-Path $Zip) { Remove-Item -Force $Zip }
New-Item -ItemType Directory -Force -Path $Out | Out-Null

Write-Host "==> release build"
cargo build --release -p download-app
$Exe = Join-Path $Root "target\release\download-tool.exe"
if (-not (Test-Path $Exe)) {
  throw "release binary not found: $Exe"
}

Copy-Item $Exe (Join-Path $Out "download-tool.exe")

$Aria = Get-Command aria2c.exe -ErrorAction SilentlyContinue
if (-not $Aria) {
  throw "aria2c.exe not found on PATH. Download from https://github.com/aria2/aria2/releases and add to PATH."
}
Copy-Item $Aria.Source (Join-Path $Out "aria2c.exe")

# CA bundle for OpenSSL-style aria2 builds
$CaCandidates = @(
  (Join-Path $env:ProgramFiles "Git\mingw64\ssl\certs\ca-bundle.crt"),
  (Join-Path ${env:ProgramFiles(x86)} "Git\mingw64\ssl\certs\ca-bundle.crt")
)
$CaCopied = $false
foreach ($ca in $CaCandidates) {
  if ($ca -and (Test-Path $ca)) {
    Copy-Item $ca (Join-Path $Out "curl-ca-bundle.crt")
    $CaCopied = $true
    break
  }
}
if (-not $CaCopied) {
  Write-Host "Fetching cacert.pem from curl.se ..."
  Invoke-WebRequest -Uri "https://curl.se/ca/cacert.pem" -OutFile (Join-Path $Out "cacert.pem")
}

@"
Download Tool (portable)

Usage:
1. Unzip this folder anywhere (e.g. Desktop)
2. Double-click download-tool.exe — no installer
3. First run registers dltool:// (current user) and login autostart if enabled

Keep these next to download-tool.exe:
- aria2c.exe
- cacert.pem or curl-ca-bundle.crt
"@ | Set-Content -Encoding UTF8 (Join-Path $Out "README.txt")

Compress-Archive -Path $Out -DestinationPath $Zip -Force

Write-Host ""
Write-Host "Folder: $Out"
Write-Host "Zip:    $Zip"
Write-Host "Unzip and double-click download-tool.exe — no installer needed."

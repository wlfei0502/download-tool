# Build a portable Windows folder + zip. No installer.
# Run on Windows (PowerShell), with Rust and aria2c available.
# Output: dist/windows/download-tool/  and  dist/windows/download-tool-portable.zip

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

$Dist = Join-Path $Root "dist\windows"
$Out = Join-Path $Dist "download-tool"
$Zip = Join-Path $Dist "download-tool-portable.zip"

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
下载器（便携版）

使用方法：
1. 解压本文件夹到任意目录（如桌面）
2. 双击 download-tool.exe 即可使用，无需安装
3. 首次运行会注册 dltool://（当前用户），并写入登录启动（如已启用）

依赖文件需与 download-tool.exe 放在同一目录：
- aria2c.exe
- cacert.pem 或 curl-ca-bundle.crt
"@ | Set-Content -Encoding UTF8 (Join-Path $Out "使用说明.txt")

Compress-Archive -Path $Out -DestinationPath $Zip -Force

Write-Host ""
Write-Host "Folder: $Out"
Write-Host "Zip:    $Zip"
Write-Host "解压后双击 download-tool.exe 即可，无需安装。"

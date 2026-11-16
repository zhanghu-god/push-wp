[CmdletBinding()]
param(
  [switch]$SkipBuild,
  [string]$Toolchain = "stable-x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
node (Join-Path $Root "..\scripts\sync-ui.mjs")
$Config = Get-Content -Raw -LiteralPath (Join-Path $Root "src-tauri\tauri.conf.json") | ConvertFrom-Json
$Version = $Config.version
$ReleaseDir = Join-Path $Root "release"
$Zip = Join-Path $ReleaseDir "push-wp_${Version}_x64-portable.zip"
$WorkDir = Join-Path $ReleaseDir ("staging-" + [guid]::NewGuid().ToString("N"))
$Stage = Join-Path $WorkDir $Config.productName

try {
  if (-not $SkipBuild) {
    rustup run $Toolchain cargo build --release --manifest-path (Join-Path $Root "src-tauri\Cargo.toml")
    if ($LASTEXITCODE -ne 0) { throw "Rust release 构建失败" }
  }
  $meta = rustup run $Toolchain cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $Root "src-tauri\Cargo.toml") | ConvertFrom-Json
  $Exe = Join-Path $meta.target_directory "release\push-wp.exe"
  if (-not (Test-Path -LiteralPath $Exe)) { throw "找不到 release 程序：$Exe" }

  New-Item -ItemType Directory -Force -Path $Stage | Out-Null
  Copy-Item -LiteralPath $Exe -Destination (Join-Path $Stage "push-wp.exe")
  Copy-Item -LiteralPath (Join-Path $Root "README.md") -Destination (Join-Path $Stage "README.md")

  New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
  Compress-Archive -LiteralPath $Stage -DestinationPath $Zip -Force
  $Hash = Get-FileHash -Algorithm SHA256 -LiteralPath $Zip
  $Sums = Join-Path $ReleaseDir "SHA256SUMS.txt"
  [System.IO.File]::WriteAllText($Sums, "$($Hash.Hash.ToLowerInvariant())  $(Split-Path -Leaf $Zip)`r`n", (New-Object System.Text.UTF8Encoding($false)))
  "Package: $Zip"
  "Checksums: $Sums"
  "Size: $((Get-Item -LiteralPath $Zip).Length) bytes"
  "SHA256: $($Hash.Hash.ToLowerInvariant())"
}
finally {
  if (Test-Path -LiteralPath $WorkDir) {
    Remove-Item -LiteralPath $WorkDir -Recurse -Force
  }
}

# 构建发布产物：NSIS 安装包 + 便携 exe（含 SHA-256）。
param(
    [switch]$SkipInstaller
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "==> pnpm tauri build" -ForegroundColor Cyan
pnpm tauri build
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$release = "target/release"
$bundle = "$release/bundle"
$out = "artifacts"
New-Item -ItemType Directory -Force -Path $out | Out-Null

if (-not $SkipInstaller) {
    Get-ChildItem "$bundle/nsis/*.exe" | ForEach-Object {
        Copy-Item $_.FullName $out -Force
        $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash
        "$hash  $($_.Name)" | Out-File "$out/$($_.Name).sha256" -Encoding ascii
        Write-Host "packaged: $($_.Name)" -ForegroundColor Green
        Write-Host "sha256:   $hash"
    }
}

# 便携版：release exe + icons 直接可用（Tauri exe 自包含前端资源）。
Copy-Item "$release/soundbridge-app.exe" "$out/SoundBridge-portable.exe" -Force
$portableHash = (Get-FileHash "$out/SoundBridge-portable.exe" -Algorithm SHA256).Hash
"$portableHash  SoundBridge-portable.exe" | Out-File "$out/SoundBridge-portable.exe.sha256" -Encoding ascii
Write-Host "packaged: SoundBridge-portable.exe" -ForegroundColor Green
Write-Host "sha256:   $portableHash"

# 构建发布产物：NSIS 安装包 + 便携 exe（含 SHA-256）。
param(
    [switch]$SkipInstaller
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "==> pnpm tauri build" -ForegroundColor Cyan
$buildArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/build-native.ps1', '-Action', 'bundle')
if ($SkipInstaller) { $buildArgs += '-SkipInstaller' }
powershell @buildArgs
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

# Keep native inference and VC runtimes beside the portable executable.
$portable = "$out/VoiceHub-portable"
New-Item -ItemType Directory -Force -Path $portable | Out-Null
Copy-Item "$release/voicehub-app.exe" "$portable/VoiceHub.exe" -Force
Copy-Item "src-tauri/transcribe-libs/*.dll" $portable -Force
Copy-Item "vendor/sayit/native/resources" $portable -Recurse -Force
Copy-Item "vendor/sayit/LICENSE" "$portable/LICENSE-SayIt" -Force
Copy-Item "THIRD_PARTY_NOTICES.md" $portable -Force
Compress-Archive -Path "$portable/*" -DestinationPath "$out/VoiceHub-portable.zip" -Force
$portableHash = (Get-FileHash "$out/VoiceHub-portable.zip" -Algorithm SHA256).Hash
"$portableHash  VoiceHub-portable.zip" | Out-File "$out/VoiceHub-portable.zip.sha256" -Encoding ascii
Write-Host "packaged: VoiceHub-portable.zip" -ForegroundColor Green
Write-Host "sha256:   $portableHash"

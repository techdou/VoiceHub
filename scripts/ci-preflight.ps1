# 声桥 SoundBridge 一键预检（等价参考仓库的 ci-preflight）
# 检查工具链 → 前端测试 → 前端构建 → Rust 全量测试 → Rust 构建。
# 任一步失败即退出非零码。

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

function Step($name, $script) {
    Write-Host ""
    Write-Host "==> $name" -ForegroundColor Cyan
    & $script
    if ($LASTEXITCODE -ne 0 -and $null -ne $LASTEXITCODE) {
        Write-Host "FAILED: $name (exit $LASTEXITCODE)" -ForegroundColor Red
        exit 1
    }
}

Write-Host "VoiceHub preflight" -ForegroundColor Green

Step "toolchain" {
    node --version
    pnpm --version
    rustc --version
    cargo --version
}

Step "install" { pnpm install --frozen-lockfile }

Step "frontend typecheck+build" { pnpm build }

Step "frontend tests" { pnpm test }

Step "rust tests" { powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-native.ps1 -Action test }

Step "rust standalone build" { powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-native.ps1 -Action build -Standalone }

Write-Host ""
Write-Host "ALL CHECKS PASSED" -ForegroundColor Green

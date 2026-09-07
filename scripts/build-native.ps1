param(
    [ValidateSet('check','test','build','bundle')][string]$Action = 'check',
    [switch]$Standalone,
    [switch]$Release,
    [switch]$SkipInstaller
)
$ErrorActionPreference = 'Stop'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = & $vswhere -latest -products '*' -property installationPath
if (-not $vs) { throw 'Visual Studio C++ Build Tools are required' }
$devcmd = Join-Path $vs 'Common7\Tools\VsDevCmd.bat'
$environment = & cmd.exe /d /c "call `"$devcmd`" -arch=x64 -host_arch=x64 >nul && set"
if ($LASTEXITCODE -ne 0) { throw 'Failed to initialize Visual Studio environment' }
foreach ($line in $environment) {
    if ($line -match '^([^=]+)=(.*)$' -and $Matches[1] -notin @('HOME','CODEX_HOME')) {
        [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
    }
}
$env:PATH = (Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin') + ';' +
    (Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja') + ';' + $env:PATH
$env:CMAKE_GENERATOR = 'Ninja'
$env:CMAKE_BUILD_PARALLEL_LEVEL = '4'
$env:VSLANG = '1033'
$env:CL = '/utf-8 ' + $env:CL
# transcribe-cpp 0.1.3's temporary junction breaks CMake working directories on
# this host. Keep the native build in Cargo's regular target directory.
Remove-Item Env:LOCALAPPDATA -ErrorAction SilentlyContinue
Remove-Item Env:TEMP -ErrorAction SilentlyContinue
Set-Location (Split-Path $PSScriptRoot -Parent)
New-Item -ItemType Directory -Force -Path artifacts | Out-Null
$ErrorActionPreference = 'Continue'
function Invoke-VoiceHubBuild {
    if ($Action -eq 'bundle') {
        if ($SkipInstaller) { & pnpm tauri build --no-bundle } else { & pnpm tauri build }
    } else {
        $buildArgs = @($Action, '--workspace', '-j', '4')
        if ($Standalone) { $buildArgs += @('--features', 'soundbridge-app/custom-protocol') }
        if ($Release) { $buildArgs += '--release' }
        & cargo @buildArgs
    }
    $script:VoiceHubBuildExitCode = $LASTEXITCODE
}
Invoke-VoiceHubBuild 2>&1 | Tee-Object -FilePath "artifacts/native-$Action.log" | ForEach-Object {
    if ($_ -match 'error|warning:|Checking |Compiling |Finished|test result:|FAILED|running [0-9]+ tests') { Write-Host $_ }
}
exit $script:VoiceHubBuildExitCode

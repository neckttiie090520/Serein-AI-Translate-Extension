#Requires -Version 5.1
<#
.SYNOPSIS
  Serein Translate setup wizard (Windows).
.DESCRIPTION
  Checks the build toolchain, clones and builds the LTEngine translation
  server (applying the translate-context patch used for context-aware mode),
  downloads a gemma3 GGUF sized to your GPU (no Hugging Face account needed)
  and writes the config consumed by scripts\start-translate.ps1.
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\setup-translate.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\setup-translate.ps1 -CheckOnly
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\setup-translate.ps1 -Model gemma3-12b -StartEngine
#>
[CmdletBinding()]
param(
    [switch]$CheckOnly,
    [switch]$SkipBuild,
    [switch]$StartEngine,
    [ValidateSet('auto', 'gemma3-1b', 'gemma3-4b', 'gemma3-12b', 'gemma3-27b')]
    [string]$Model = 'auto',
    [string]$ModelFile = '',
    [string]$EngineDir = '',
    [int]$Port = 5050,
    [switch]$Autostart,
    [switch]$NoAutostart
)

$ErrorActionPreference = 'Stop'

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$DataDir    = Join-Path $env:LOCALAPPDATA 'SereinTranslate'
$ModelsDir  = Join-Path $DataDir 'models'
$ConfigPath = Join-Path $DataDir 'config.json'
$PatchPath  = Join-Path $PSScriptRoot 'ltengine-translate-context.patch'
$EngineUrl  = 'https://github.com/LibreTranslate/LTEngine'
$BindHost   = '127.0.0.1'

$ModelCatalog = @{
    'gemma3-1b'  = @{ repo = 'unsloth/gemma-3-1b-it-GGUF';  file = 'gemma-3-1b-it-Q4_K_M.gguf';  disk = '~0.8 GB';  minVramMb = 2000;  note = 'smoke test only' }
    'gemma3-4b'  = @{ repo = 'unsloth/gemma-3-4b-it-GGUF';  file = 'gemma-3-4b-it-Q4_K_M.gguf';  disk = '~2.5 GB';  minVramMb = 3500;  note = 'recommended default' }
    'gemma3-12b' = @{ repo = 'unsloth/gemma-3-12b-it-GGUF'; file = 'gemma-3-12b-it-Q4_K_M.gguf'; disk = '~7.5 GB';  minVramMb = 9500;  note = 'better nuance' }
    'gemma3-27b' = @{ repo = 'unsloth/gemma-3-27b-it-GGUF'; file = 'gemma-3-27b-it-Q4_K_M.gguf'; disk = '~16.5 GB'; minVramMb = 17000; note = 'best quality, slowest' }
}

function Write-Step([string]$Message) { Write-Host ''; Write-Host ('==> ' + $Message) -ForegroundColor Cyan }
function Write-Tip([string]$Message) { Write-Host ('    ' + $Message) }
function Write-Miss([string]$Message) { Write-Host ('    [missing] ' + $Message) -ForegroundColor Yellow }

function Test-Tool([string]$Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Get-GpuInfo {
    $result = @{ name = 'unknown GPU'; vramMb = -1 }
    try {
        $gpu = Get-CimInstance Win32_VideoController | Select-Object -First 1
        if ($gpu) { $result.name = $gpu.Name }
    } catch { }
    $smi = Get-Command 'nvidia-smi.exe' -ErrorAction SilentlyContinue
    if ($smi) {
        try {
            $line = (& nvidia-smi --query-gpu=name,memory.total --format=csv,noheader | Select-Object -First 1)
            $parts = ($line -split ',')
            if ($parts.Count -ge 2) {
                $result.name = $parts[0].Trim()
                if ($parts[1] -match '(\d+)') { $result.vramMb = [int]$Matches[1] }
            }
        } catch { }
    }
    return $result
}

function Pick-Model([int]$VramMb) {
    if ($VramMb -ge 17000) { return 'gemma3-27b' }
    if ($VramMb -ge 9500) { return 'gemma3-12b' }
    if ($VramMb -ge 3500) { return 'gemma3-4b' }
    if ($VramMb -ge 0) { return 'gemma3-1b' }
    return 'gemma3-4b'
}

$gpu = Get-GpuInfo
if ($Model -eq 'auto') { $Model = Pick-Model $gpu.vramMb }

Write-Host 'Serein Translate setup'
Write-Host ('  GPU: ' + $gpu.name + '  VRAM: ' + $(if ($gpu.vramMb -ge 0) { [string]$gpu.vramMb + ' MiB (nvidia-smi)' } else { 'unknown' }))
Write-Host ('  Model: ' + $Model + '  (' + $ModelCatalog[$Model].note + ')')

$tools = @('git', 'cargo', 'cmake', 'clang')
$missing = @($tools | Where-Object { -not (Test-Tool $_) })

if ($CheckOnly) {
    Write-Step 'Environment report'
    foreach ($t in $tools) {
        if (Test-Tool $t) { Write-Tip ($t + ': ok') } else { Write-Miss ($t + ': not found') }
    }
    Write-Tip ('model would be: ' + $Model + ' -> ' + (Join-Path $ModelsDir $ModelCatalog[$Model].file))
    if ($EngineDir) { Write-Tip ('engine dir: ' + $EngineDir) } else { Write-Tip ('engine dir: auto (sibling LTEngine if present, else ' + (Join-Path $DataDir 'LTEngine') + ')') }
    if ($missing.Count -gt 0) {
        Write-Host ''
        Write-Host 'Install hints for missing tools:' -ForegroundColor Yellow
        Write-Host '  winget install Git.Git'
        Write-Host '  winget install Rustlang.Rustup   # then open a new terminal'
        Write-Host '  winget install Kitware.CMake'
        Write-Host '  winget install LLVM.LLVM         # clang for the llama.cpp bindings'
        Write-Host '  Visual Studio 2022 Build Tools with the Desktop development with C++ workload (MSVC linker)'
    }
    exit 0
}

if ($missing.Count -gt 0) {
    Write-Host ('Missing tools: ' + ($missing -join ', ')) -ForegroundColor Red
    Write-Host 'Rerun with -CheckOnly to see install hints.'
    exit 1
}

if (-not $EngineDir) {
    $sibling = Join-Path (Split-Path -Parent $RepoRoot) 'LTEngine'
    if (Test-Path (Join-Path $sibling 'Cargo.toml')) { $EngineDir = $sibling } else { $EngineDir = Join-Path $DataDir 'LTEngine' }
}

if (-not (Test-Path (Join-Path $EngineDir 'Cargo.toml'))) {
    Write-Step ('Cloning LTEngine into ' + $EngineDir)
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $EngineDir) | Out-Null
    & git clone --depth 1 $EngineUrl $EngineDir
    if ($LASTEXITCODE -ne 0) { throw 'git clone of LTEngine failed' }
}

$mainSrc = Join-Path $EngineDir 'ltengine\src\main.rs'
if ((Test-Path $PatchPath) -and (Test-Path $mainSrc) -and -not (Select-String -Path $mainSrc -Pattern 'translate-context' -Quiet)) {
    Write-Step 'Applying translate-context patch (context-aware mode endpoint)'
    & git -C $EngineDir apply --check $PatchPath
    if ($LASTEXITCODE -ne 0) {
        Write-Miss 'patch does not apply to this LTEngine revision; continuing without it (context-aware mode falls back to plain /translate)'
    } else {
        & git -C $EngineDir apply $PatchPath
        Write-Tip 'patched /translate-context into the engine'
    }
}

$engineExe = Join-Path $EngineDir 'target\release\ltengine.exe'

if ($SkipBuild) {
    if (-not (Test-Path $engineExe)) { throw 'No existing engine build found; rerun without -SkipBuild.' }
    Write-Step 'Reusing existing engine build'
} else {
    Write-Step 'Building LTEngine (cargo build --release --features vulkan)'
    Write-Tip 'first build compiles llama.cpp and can take 10-30 minutes'
    Push-Location $EngineDir
    try { & cargo build --release --features vulkan } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'engine build failed (see output above)' }
}

if (-not $ModelFile) { $ModelFile = Join-Path $ModelsDir $ModelCatalog[$Model].file }

if ((-not (Test-Path $ModelFile)) -or ((Get-Item $ModelFile).Length -lt 100MB)) {
    Write-Step ('Downloading ' + $Model + ' GGUF (' + $ModelCatalog[$Model].disk + ') - resumable, no Hugging Face account needed')
    New-Item -ItemType Directory -Force -Path $ModelsDir | Out-Null
    $url = 'https://huggingface.co/' + $ModelCatalog[$Model].repo + '/resolve/main/' + $ModelCatalog[$Model].file
    & curl.exe -L -C - --retry 3 --retry-delay 2 -o $ModelFile $url
    if ($LASTEXITCODE -ne 0) { throw 'model download failed; rerun the wizard to resume' }
} else {
    Write-Step 'Model already downloaded'
}

New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
$config = [ordered]@{
    engineExe = $engineExe
    modelFile = $ModelFile
    host      = $BindHost
    port      = $Port
    model     = $Model
}
ConvertTo-Json $config | Set-Content -Path $ConfigPath -Encoding ASCII

Write-Step 'Background autostart (recommended)'
if ($NoAutostart) {
    Write-Tip 'skipped (-NoAutostart)'
} elseif ($Autostart) {
    & (Join-Path $PSScriptRoot 'start-translate.ps1') -Install
} else {
    $answer = Read-Host '    Install autostart at login? The engine then runs hidden in the background [Y/n]'
    if ([string]::IsNullOrWhiteSpace($answer) -or $answer -match '^[yY]') {
        & (Join-Path $PSScriptRoot 'start-translate.ps1') -Install
    } else {
        Write-Tip 'skipped; add it any time:  powershell -File scripts\start-translate.ps1 -Install'
    }
}

Write-Step 'Done'
Write-Tip ('config written to ' + $ConfigPath)
Write-Tip 'the engine starts hidden, in the background; it has no window or taskbar button'
Write-Tip 'manage:  powershell -File scripts\start-translate.ps1 -Status / -Stop / -Uninstall'
Write-Tip ('engine log: ' + (Join-Path $DataDir 'engine.log'))
Write-Tip 'then run the client from the repo root:  cargo run --release'
if ($StartEngine) { & (Join-Path $PSScriptRoot 'start-translate.ps1') }


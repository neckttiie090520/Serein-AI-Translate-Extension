#Requires -Version 5.1
<#
.SYNOPSIS
  Start or stop the local LTEngine translation server using the config
  written by setup-translate.ps1.
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\start-translate.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\start-translate.ps1 -Stop
#>
[CmdletBinding()]
param(
    [switch]$HelpOnly,
    [switch]$Stop,
    [int]$Port = 0
)

if ($HelpOnly) {
    Write-Host 'Usage: powershell -File scripts\start-translate.ps1 [-Port <n>] [-Stop]'
    Write-Host 'Reads %LOCALAPPDATA%\SereinTranslate\config.json (written by setup-translate.ps1).'
    Write-Host 'The Serein client connects to 127.0.0.1:5050 by default (SEREIN_TRANSLATE_HOST to override).'
    exit 0
}

$ConfigPath = Join-Path $env:LOCALAPPDATA 'SereinTranslate\config.json'
if (-not (Test-Path $ConfigPath)) {
    Write-Host 'No config found. Run scripts\setup-translate.ps1 first.' -ForegroundColor Red
    exit 1
}
$config = Get-Content $ConfigPath -Raw | ConvertFrom-Json
if ($Port -le 0) { $Port = [int]$config.port }

function Test-Port([string]$HostName, [int]$PortNumber) {
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $async = $client.BeginConnect($HostName, $PortNumber, $null, $null)
        return ($async.AsyncWaitHandle.WaitOne(700) -and $client.Connected)
    } catch {
        return $false
    } finally {
        $client.Close()
    }
}

if ($Stop) {
    Get-Process ltengine -ErrorAction SilentlyContinue | Stop-Process -Force
    Write-Host 'Engine stopped.'
    exit 0
}

if (Test-Port $config.host $Port) {
    Write-Host ('Engine already listening on ' + $config.host + ':' + $Port)
    exit 0
}

Write-Host 'Starting LTEngine and loading the model (this can take up to a minute)...'
$engineArgs = ('--host "{0}" --port {1} --model-file "{2}"' -f $config.host, $Port, $config.modelFile)
$process = Start-Process -FilePath $config.engineExe -ArgumentList $engineArgs -WindowStyle Minimized -PassThru

$deadline = (Get-Date).AddSeconds(180)
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 2
    if ($process.HasExited) {
        Write-Host 'Engine exited immediately. Run it manually to see the error:' -ForegroundColor Red
        Write-Host ('  ' + $config.engineExe + ' ' + $engineArgs)
        exit 1
    }
    if (Test-Port $config.host $Port) {
        Write-Host ('Translation engine ready on ' + $config.host + ':' + $Port)
        Write-Host 'Now run the Serein client from the repo root: cargo run --release'
        exit 0
    }
}
Write-Host 'Timed out waiting for the engine port. Check the engine window or logs.' -ForegroundColor Red
exit 1


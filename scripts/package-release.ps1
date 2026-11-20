# Package a Windows release zip with the layout the in-app updater requires.
#
# The updater's installation check (apps/desktop/src/updater_install.rs
# `installation()`) only recognizes an "extracted release package" when the
# exe folder contains THIRD_PARTY_NOTICES.md and a licenses/ directory, and
# the repack step only accepts files whose top-level zip path is on the
# WINDOWS_FILES allowlist. An exe-only zip downloads fine but is REFUSED at
# install time - this script exists so that can never happen again.
#
# Usage:  powershell -File scripts\package-release.ps1 -Version 0.3.1
#         (run after `cargo build --release -p serein`; prints the zip and
#          checksum paths, ready for `gh release create/upload`)

param(
	[Parameter(Mandatory = $true)]
	[string]$Version
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.IO.Compression
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target\release\serein.exe'
if (-not (Test-Path $exe)) {
	throw 'target\release\serein.exe not found - build first: cargo build --release -p serein'
}
foreach ($required in @('README.md', 'THIRD_PARTY_NOTICES.md', 'LICENSE-MIT', 'LICENSE-APACHE')) {
	if (-not (Test-Path (Join-Path $repo $required))) {
		throw "release package file missing from repo root: $required"
	}
}
$licensesDir = Join-Path $repo 'licenses'
if (-not (Test-Path $licensesDir)) {
	throw 'licenses\ directory missing from repo root'
}

$stage = Join-Path $env:TEMP "serein-package-v$Version"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage | Out-Null

Copy-Item $exe (Join-Path $stage 'serein.exe')
Copy-Item (Join-Path $repo 'README.md') $stage
Copy-Item (Join-Path $repo 'THIRD_PARTY_NOTICES.md') $stage
Copy-Item (Join-Path $repo 'LICENSE-MIT') $stage
Copy-Item (Join-Path $repo 'LICENSE-APACHE') $stage
Copy-Item $licensesDir (Join-Path $stage 'licenses') -Recurse

$zipName = "serein-v$Version-Windows-X64.zip"
$zipPath = Join-Path $stage $zipName
# Compress-Archive on PowerShell 5.1 writes backslash entry names; the zip
# spec (and the updater's macOS branch) expects forward slashes. Create the
# archive by hand with explicit `/` separators.
$files = Get-ChildItem $stage -Recurse -File | Where-Object { $_.Name -ne $zipName }
$stream = [System.IO.File]::Open($zipPath, [System.IO.FileMode]::Create)
$archive = New-Object System.IO.Compression.ZipArchive($stream, [System.IO.Compression.ZipArchiveMode]::Create)
foreach ($file in $files) {
	$rel = $file.FullName.Substring($stage.Length + 1).Replace('\', '/')
	$entry = $archive.CreateEntry($rel, [System.IO.Compression.CompressionLevel]::Optimal)
	$src = [System.IO.File]::OpenRead($file.FullName)
	$out = $entry.Open()
	$src.CopyTo($out)
	$out.Dispose()
	$src.Dispose()
}
$archive.Dispose()
$stream.Dispose()

# Verify the layout the updater checks for: notices file + licenses dir at the
# zip root, plus every top-level entry on the repack allowlist. The updater
# splits on path components, so accept both separators when auditing.
$archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
$top = $archive.Entries | ForEach-Object { ($_.FullName -split '[\\/]')[0] } | Sort-Object -Unique
$archive.Dispose()
$allowed = @('serein.exe', 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE',
	'THIRD_PARTY_NOTICES.md', 'licenses', 'docs', 'source',
	'install-notifications.ps1', 'setup.ps1')
foreach ($name in $top) {
	if ($allowed -notcontains $name) {
		throw "zip contains a top-level entry the updater repack would refuse: $name"
	}
}
if ($top -notcontains 'serein.exe' -or $top -notcontains 'THIRD_PARTY_NOTICES.md' -or $top -notcontains 'licenses') {
	throw 'zip is missing serein.exe, THIRD_PARTY_NOTICES.md or licenses/ - updater would refuse to install'
}

$hash = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
"$hash  $zipName" | Set-Content (Join-Path $stage 'SHA256SUMS.txt') -Encoding ascii

Write-Output ('zip:   ' + $zipPath)
Write-Output ('bytes: ' + (Get-Item $zipPath).Length)
Write-Output ('sha256 ' + $hash)
Write-Output ('sums:  ' + (Join-Path $stage 'SHA256SUMS.txt'))
Write-Output 'layout verified: serein.exe + notices + licenses/ present, all entries on the repack allowlist'

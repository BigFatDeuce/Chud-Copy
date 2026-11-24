# Publie une version sur GitHub : les installateurs comme fichiers de Release, puis la presentation
# (README.md de ce dossier) avec le code source sous GPL-3.0 (publish-source.ps1). Le jeton vient de Git Credential Manager
# et reste en memoire.
#
# Usage : powershell -ExecutionPolicy Bypass -File publication\publish-release.ps1 -Version 1.8.4 -Notes publication\notes-1.8.4.md -Title "short English summary"
# Prerequis : les cinq fichiers de la version dans release\ (voir NOUVEAU-PC.md).
param([Parameter(Mandatory)][string]$Version, [Parameter(Mandatory)][string]$Notes, [Parameter(Mandatory)][string]$Title)
$Project = Split-Path $PSScriptRoot -Parent
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$owner = 'BigFatDeuce'; $repo = 'Chud-Copy'; $version = $Version; $tag = "v$version"
$assets = @(
  @{ From = "Chud Copy_${version}_x64-setup.exe";    Name = "Chud-Copy_${version}_x64-setup.exe" },
  @{ From = "Chud Copy_${version}_x64-portable.zip"; Name = "Chud-Copy_${version}_x64-portable.zip" },
  @{ From = "Chud Copy_${version}_amd64.AppImage";   Name = "Chud-Copy_${version}_amd64.AppImage" },
  @{ From = "Chud Copy-${version}-1.x86_64.rpm";     Name = "Chud-Copy-${version}-1.x86_64.rpm" },
  @{ From = "Chud Copy_${version}_amd64.deb";       Name = "Chud-Copy_${version}_amd64.deb" }
)
$request = Join-Path $env:TEMP 'chud-credential-request.txt'
[IO.File]::WriteAllText($request, "protocol=https`nhost=github.com`n`n", (New-Object Text.UTF8Encoding $false))
$credential = cmd /c "git credential fill < `"$request`""
Remove-Item $request -Force
$token = (($credential | Where-Object { $_ -match '^password=' }) -replace '^password=', '')
if (-not $token) { throw 'Aucune connexion GitHub obtenue.' }
$headers = @{ Authorization = "Bearer $token"; Accept = 'application/vnd.github+json'; 'User-Agent' = 'chud-copy-release' }

# Notes en anglais, en Markdown : les accents graves y sont ecrits normalement.
$notes = [IO.File]::ReadAllText((Resolve-Path $Notes)).Trim()
foreach ($asset in $assets) { if (-not (Test-Path -LiteralPath (Join-Path $Project "release\$($asset.From)"))) { throw "Fichier absent : release\$($asset.From)" } }

$hashes = foreach ($asset in $assets) {
  $hash = (Get-FileHash -LiteralPath (Join-Path $Project "release\$($asset.From)") -Algorithm SHA256).Hash.ToLower()
  "| ``$($asset.Name)`` | ``$hash`` |"
}
$body = @"
Chud Copy is a local, manual copy and sync tool for Windows and Linux.

**Windows:** ``Chud-Copy_${version}_x64-setup.exe`` (installer) or ``Chud-Copy_${version}_x64-portable.zip`` (no installation). The app is not code-signed yet, so SmartScreen may show a warning.

**Fedora, Nobara:** ``Chud-Copy-${version}-1.x86_64.rpm``, install it with ``sudo dnf install ./Chud-Copy-${version}-1.x86_64.rpm``. Recommended on these systems.

**Debian, Ubuntu, Linux Mint:** ``Chud-Copy_${version}_amd64.deb``, install it with ``sudo apt install ./Chud-Copy_${version}_amd64.deb``. Recommended on these systems.

**Other Linux:** ``Chud-Copy_${version}_amd64.AppImage``, make it executable and run it.

## What's new

$notes
| File | SHA-256 |
| --- | --- |
$($hashes -join "`n")
"@
$payload = @{ tag_name = $tag; target_commitish = 'main'; name = "Chud Copy $version"; body = $body; draft = $false; prerelease = $false } | ConvertTo-Json
$release = Invoke-RestMethod -UseBasicParsing -Method Post -Headers $headers -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($payload)) -Uri "https://api.github.com/repos/$owner/$repo/releases"
Write-Output "Release creee : $($release.html_url)"
foreach ($asset in $assets) {
  $path = Join-Path $Project "release\$($asset.From)"
  $url = "https://uploads.github.com/repos/$owner/$repo/releases/$($release.id)/assets?name=$($asset.Name)"
  $uploaded = Invoke-RestMethod -UseBasicParsing -Method Post -Headers $headers -ContentType 'application/octet-stream' -InFile $path -Uri $url -TimeoutSec 1800
  if ($uploaded.size -ne (Get-Item -LiteralPath $path).Length) { throw "Taille incorrecte pour $($asset.Name)." }
  Write-Output "  ok : $($asset.Name)"
}
Remove-Variable token, credential, headers

# Code source et presentation (README) : voir publish-source.ps1.
& (Join-Path $PSScriptRoot 'publish-source.ps1') -Message "Chud Copy ${version}: $Title"
Write-Output 'Publication terminee.'

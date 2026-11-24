# Publie le code source sur le depot GitHub public, sous GPL-3.0-or-later : tous les fichiers suivis par Git
# dans ce dossier, dans leur etat commite (HEAD), avec publication\README.md comme README.md. Les captures de
# .github/assets restent celles du depot public. Les notes d'assistant (CLAUDE.md, AGENTS.md, .claude/...) et
# tout fichier contenant un mot de la liste locale publication\.private-words (ignoree par Git : nom, e-mail,
# nom du PC, chemins locaux) ne quittent jamais cet ordinateur : la publication s'arrete avant le push.
#
# Usage : powershell -ExecutionPolicy Bypass -File publication\publish-source.ps1 -Message "Publish the source code"
# Appele aussi a la fin de publish-release.ps1. Ne cree ni release ni tag : les mises a jour ne bougent pas.
param([Parameter(Mandatory)][string]$Message)
$Project = Split-Path $PSScriptRoot -Parent
$ErrorActionPreference = 'Stop'
$owner = 'BigFatDeuce'; $repo = 'Chud-Copy'
if (git -C $Project status --porcelain) { throw 'Des changements ne sont pas commites : seul l''etat commite est publie.' }
$words = Join-Path $PSScriptRoot '.private-words'
if (-not (Test-Path -LiteralPath $words)) { throw 'publication\.private-words manque (un mot prive par ligne).' }

$clone = Join-Path $env:TEMP 'chud-copy-public'
if (Test-Path $clone) { cmd /c "rmdir /s /q `"$clone`"" }
cmd /c "git clone --quiet https://github.com/$owner/$repo.git `"$clone`" 2>&1"
if ($LASTEXITCODE -ne 0) { throw 'clone impossible' }
# Vide le clone (sauf .git et .github), puis y depose l'arbre commite.
Get-ChildItem -LiteralPath $clone -Force | Where-Object { $_.Name -notin '.git', '.github' } | Remove-Item -Recurse -Force
$archive = Join-Path $env:TEMP 'chud-copy-source.tar'
git -C $Project archive --format=tar -o $archive HEAD
tar -xf $archive -C $clone
Remove-Item $archive
Copy-Item (Join-Path $PSScriptRoot 'README.md') (Join-Path $clone 'README.md') -Force

Push-Location $clone
try {
  git config user.name $owner
  git config user.email "67258247+$owner@users.noreply.github.com"
  git config core.autocrlf true
  cmd /c "git add -A 2>nul"
  $blocked = git ls-files | Where-Object { $_ -match '(^|/)(CLAUDE(\.local)?\.md|AGENTS\.md|NOUVEAU-PC\.md|\.claude/|\.env|\.private-words)' -or $_ -match '\.(pem|key|pfx|p12|jks|keystore)$' }
  if ($blocked) { throw "Fichiers interdits : $blocked" }
  $private = git grep --cached -I -i -l -F -f $words
  if ($private) { throw "Donnees privees trouvees dans : $private" }
  $changed = @(git diff --cached --name-only)
  if (-not $changed.Count) { Write-Output 'Rien a publier.'; return }
  Write-Output "$($changed.Count) fichier(s) modifie(s)"
  $file = Join-Path $env:TEMP 'chud-copy-commit.txt'
  [IO.File]::WriteAllText($file, "$Message`n`nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`n", (New-Object Text.UTF8Encoding $false))
  $env:TZ = 'UTC'
  cmd /c "git commit -q -F `"$file`" 2>&1"
  Remove-Item $file
  cmd /c "git push --quiet origin main 2>&1"
  if ($LASTEXITCODE -ne 0) { throw 'push refuse' }
  Write-Output "Code publie : $(git log -1 --format=%h)"
} finally { Pop-Location }

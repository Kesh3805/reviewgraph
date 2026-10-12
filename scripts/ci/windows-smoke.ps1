param([Parameter(Mandatory = $true)][string]$Binary)
$ErrorActionPreference = 'Stop'

function Invoke-Checked([string[]]$CliArgs) {
  & $Binary @CliArgs
  if ($LASTEXITCODE -ne 0) { throw "review $($CliArgs -join ' ') exited $LASTEXITCODE" }
}

Invoke-Checked @('--version')

# Fixture repository in a path containing spaces.
$repo = Join-Path $env:RUNNER_TEMP 'smoke repo'
New-Item -ItemType Directory -Force $repo | Out-Null
git -C $repo init -q
Set-Content -Path (Join-Path $repo 'a.ts') -Value "export const a = 1;`n"
git -C $repo add -A
git -C $repo -c user.name=ci -c user.email=ci@example.invalid commit -q -m init

Invoke-Checked @('init', '--repository', $repo)
if (-not (Test-Path (Join-Path $repo '.review/repository.json'))) { throw '.review/repository.json missing' }
Invoke-Checked @('status', '--repository', $repo)
Write-Host 'windows smoke ok'

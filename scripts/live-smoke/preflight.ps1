# Live-smoke preflight (E2E-002), PowerShell variant. Prints set/missing, never a value.
# Exit codes: 0 ok, 1 missing variable, 2 tunnel unreachable, 3 model key rejected.
$envFile = if ($env:LIVE_SMOKE_ENV_FILE) { $env:LIVE_SMOKE_ENV_FILE } else { '.env.live' }
if (Test-Path $envFile) {
  Get-Content $envFile | ForEach-Object {
    if ($_ -match '^\s*([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
      Set-Item -Path "Env:$($Matches[1])" -Value $Matches[2].Trim('"')
    }
  }
}

$status = 0
function Test-Var([string]$name) {
  if ([Environment]::GetEnvironmentVariable($name)) { "${name}: set" }
  else { "${name}: missing"; $script:status = 1 }
}

foreach ($n in 'GITHUB_APP_ID', 'GITHUB_WEBHOOK_SECRET', 'LIVE_SMOKE_TUNNEL_URL') { Test-Var $n }
if ($env:GITHUB_APP_PRIVATE_KEY -or $env:GITHUB_APP_PRIVATE_KEY_FILE) { 'GITHUB_APP_PRIVATE_KEY(_FILE): set' }
else { 'GITHUB_APP_PRIVATE_KEY(_FILE): missing'; $status = 1 }
if ($env:ANTHROPIC_API_KEY) { 'ANTHROPIC_API_KEY: set' }
elseif ($env:OPENAI_API_KEY) { 'OPENAI_API_KEY: set' }
else { 'ANTHROPIC_API_KEY|OPENAI_API_KEY: missing'; $status = 1 }

if ($status -ne 0) { 'preflight failed: missing variables'; exit 1 }

try {
  Invoke-WebRequest -Uri ($env:LIVE_SMOKE_TUNNEL_URL.TrimEnd('/') + '/health') -TimeoutSec 10 -UseBasicParsing | Out-Null
  'tunnel: reachable'
} catch { 'tunnel: unreachable'; exit 2 }

if ($env:LIVE_SMOKE_SKIP_MODEL_CALL -eq '1') { 'model key: skipped' }
else {
  try {
    if ($env:ANTHROPIC_API_KEY) {
      $model = if ($env:LIVE_SMOKE_MODEL) { $env:LIVE_SMOKE_MODEL } else { 'claude-haiku-4-5' }
      $body = @{ model = $model; max_tokens = 1; messages = @(@{ role = 'user'; content = 'hi' }) } | ConvertTo-Json -Depth 5
      Invoke-WebRequest -Uri 'https://api.anthropic.com/v1/messages' -Method Post -Body $body -ContentType 'application/json' `
        -Headers @{ 'x-api-key' = $env:ANTHROPIC_API_KEY; 'anthropic-version' = '2023-06-01' } -UseBasicParsing | Out-Null
    } else {
      $model = if ($env:LIVE_SMOKE_MODEL) { $env:LIVE_SMOKE_MODEL } else { 'gpt-4o-mini' }
      $body = @{ model = $model; max_tokens = 1; messages = @(@{ role = 'user'; content = 'hi' }) } | ConvertTo-Json -Depth 5
      Invoke-WebRequest -Uri 'https://api.openai.com/v1/chat/completions' -Method Post -Body $body -ContentType 'application/json' `
        -Headers @{ Authorization = "Bearer $($env:OPENAI_API_KEY)" } -UseBasicParsing | Out-Null
    }
    'model key: ok'
  } catch { 'model key: rejected'; exit 3 }
}
'preflight passed'

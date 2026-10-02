# PowerShell entry point: forwards to cargo.sh via Git Bash.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
& bash "$here/cargo.sh" @args
exit $LASTEXITCODE

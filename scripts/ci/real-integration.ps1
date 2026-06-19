$ErrorActionPreference = "Stop"

$root = Join-Path $env:RUNNER_TEMP "dual-integration"
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $root | Out-Null
Set-Location $root

& $env:DUAL_BIN init --name ci-integration
& $env:DUAL_BIN add r jsonlite reticulate
& $env:DUAL_BIN add py "six==1.17.0"

$config = Get-Content dual.toml -Raw
$tasks = @'
[tasks]
rcheck = 'Rscript -e "cat(jsonlite::toJSON(list(ok=TRUE)))"'
pycheck = 'python -c "import six; print(six.__version__)"'
'@
$config = $config.Replace("[tasks]", $tasks.Trim())
Set-Content dual.toml $config

& $env:DUAL_BIN --trust-project up
& $env:DUAL_BIN doctor
& $env:DUAL_BIN run rcheck
& $env:DUAL_BIN run pycheck
if (-not (Test-Path dual.lock)) { throw "dual.lock was not created" }

New-Item -ItemType Directory -Path scripts/nested -Force | Out-Null
Set-Location scripts/nested
& $env:DUAL_BIN --trust-project doctor
Set-Location ../..

& $env:DUAL_BIN clean --yes
if (-not (Test-Path dual.lock)) { throw "dual.lock was removed" }
if (Test-Path .dual) { throw ".dual was not cleaned" }

& $env:DUAL_BIN up
$doctor = & $env:DUAL_BIN doctor
if (-not ($doctor -match "reticulate uses the project Python")) {
    throw "reticulate did not use the project Python"
}
& $env:DUAL_BIN clean --yes

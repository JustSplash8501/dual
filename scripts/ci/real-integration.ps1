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

& $env:DUAL_BIN init --script report.qmd --python 3.12
& $env:DUAL_BIN add --script report.qmd --python matplotlib
& $env:DUAL_BIN --trust-project run report.qmd
if (-not (Test-Path report.html)) { throw "Quarto did not create report.html" }
$report = Get-Content report.html -Raw
if (-not ($report -match "Hello from dual")) {
    throw "Quarto output did not contain the executed document content"
}

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

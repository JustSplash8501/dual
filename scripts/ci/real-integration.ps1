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

# Project-backed Quarto metadata must merge with dual.toml without replacing
# the project's environment manifest or shared lockfile.
$projectManifestHash = (Get-FileHash ".dual/workspace/pyproject.toml" -Algorithm SHA256).Hash
$projectLockHash = (Get-FileHash "dual.lock" -Algorithm SHA256).Hash
& $env:DUAL_BIN init --script report.qmd --python 3.12
& $env:DUAL_BIN add --script report.qmd --python matplotlib
& $env:DUAL_BIN --trust-project run report.qmd
if (-not (Test-Path report.html)) { throw "Quarto did not create report.html" }
$report = Get-Content report.html -Raw
if (-not ($report -match "Hello from dual")) {
    throw "Quarto output did not contain the executed document content"
}
if ((Get-FileHash ".dual/workspace/pyproject.toml" -Algorithm SHA256).Hash -ne $projectManifestHash) {
    throw "Script rendering replaced the project environment manifest"
}
if ((Get-FileHash "dual.lock" -Algorithm SHA256).Hash -ne $projectLockHash) {
    throw "Script rendering replaced the project lockfile"
}
& $env:DUAL_BIN --trust-project doctor | Out-Null

# A synchronized script must run without installing again.
@'
print("dual no-install ok")
'@ | Set-Content executable.py
& $env:DUAL_BIN init --script executable.py --python 3.12
& $env:DUAL_BIN --trust-project sync --script executable.py
$noInstall = & $env:DUAL_BIN run executable.py --no-install
if (-not ($noInstall -match "dual no-install ok")) {
    throw "Synchronized script did not run with --no-install"
}

# Standalone documents must work without an ancestor dual.toml.
$projectRoot = $root
$standaloneQuarto = Join-Path $env:RUNNER_TEMP "dual-standalone-quarto"
Remove-Item -Recurse -Force $standaloneQuarto -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $standaloneQuarto | Out-Null
Set-Location $standaloneQuarto
& $env:DUAL_BIN init --script standalone.qmd --python 3.12
& $env:DUAL_BIN --trust-project run standalone.qmd
if (-not (Test-Path standalone.html)) { throw "Standalone Quarto did not render" }
if (-not ((Get-Content standalone.html -Raw) -match "Hello from dual")) {
    throw "Standalone Quarto output did not contain executed content"
}

$standaloneRMarkdown = Join-Path $env:RUNNER_TEMP "dual-standalone-rmarkdown"
Remove-Item -Recurse -Force $standaloneRMarkdown -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $standaloneRMarkdown | Out-Null
Set-Location $standaloneRMarkdown
& $env:DUAL_BIN init --script standalone.Rmd --r 4.5
& $env:DUAL_BIN --trust-project run standalone.Rmd
if (-not (Test-Path standalone.html)) { throw "Standalone R Markdown did not render" }
if (-not ((Get-Content standalone.html -Raw) -match "Hello from dual")) {
    throw "Standalone R Markdown output did not contain executed content"
}
Set-Location $projectRoot

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

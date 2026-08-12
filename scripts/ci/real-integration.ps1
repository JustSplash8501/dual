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
renvcheck = 'Rscript -e "cat(Sys.getenv(\"PROJECT_SHARED_ENV\"))"'
pyenvcheck = 'python -c "import os; print(os.getenv(\"PROJECT_SHARED_ENV\"))"'
'@
$config = $config.Replace("[tasks]", $tasks.Trim())
Set-Content dual.toml $config
'PROJECT_SHARED_ENV=dotenv-ok' | Set-Content .env

& $env:DUAL_BIN --trust-project up
$cacheInfo = (& $env:DUAL_BIN --json cache info) -join "`n"
if (-not ($cacheInfo -match '"exists": true')) { throw "Dual cache was not created" }
if (-not (Test-Path (Join-Path $env:DUAL_CACHE_DIR ".dual-cache"))) {
    throw "Dual cache marker was not created"
}
if (-not (Test-Path (Join-Path $env:DUAL_CACHE_DIR "v1/environment"))) {
    throw "Dual environment cache bucket was not created"
}
& $env:DUAL_BIN doctor
& $env:DUAL_BIN run rcheck
& $env:DUAL_BIN run pycheck
$rDotenv = (& $env:DUAL_BIN run renvcheck) -join "`n"
if (-not ($rDotenv -match "dotenv-ok")) { throw "R did not inherit project .env" }
$pythonDotenv = (& $env:DUAL_BIN run pyenvcheck) -join "`n"
if (-not ($pythonDotenv -match "dotenv-ok")) { throw "Python did not inherit project .env" }
if (-not (Test-Path dual.lock)) { throw "dual.lock was not created" }

# Project initialization must produce complete, runnable single-language
# environments without installing or reporting the omitted runtime.
$mixedRoot = $root
$pythonOnly = Join-Path $env:RUNNER_TEMP "dual-python-only"
Remove-Item -Recurse -Force $pythonOnly -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $pythonOnly | Out-Null
Set-Location $pythonOnly
& $env:DUAL_BIN init python-only --python 3.12
& $env:DUAL_BIN add py "six==1.17.0"
$config = Get-Content dual.toml -Raw
$pythonTask = @'
[tasks]
pycheck = 'python -c "import six; print(six.__version__)"'
'@
$config = $config.Replace("[tasks]", $pythonTask.Trim())
Set-Content dual.toml $config
'cat("R enabled")' | Set-Content scripts/enabled.R
$pythonUp = (& $env:DUAL_BIN --trust-project up) -join "`n"
if (-not ($pythonUp -match "Python 3.12 requested")) {
    throw "Python-only preparation did not report Python"
}
if ($pythonUp -match "R packages configured") {
    throw "Python-only preparation reported R packages"
}
$pythonRun = (& $env:DUAL_BIN run pycheck) -join "`n"
if (-not ($pythonRun -match "1.17.0")) { throw "Python-only task failed" }
$pythonDoctor = (& $env:DUAL_BIN doctor) -join "`n"
if (-not ($pythonDoctor -match "R is not required by this environment")) {
    throw "Python-only doctor report treated R as enabled"
}
if (-not (Test-Path dual.lock)) { throw "Python-only lockfile was not created" }
$pythonManifest = Get-Content ".dual/workspace/pyproject.toml" -Raw
if (-not ($pythonManifest -match "(?m)^python = ")) {
    throw "Python-only manifest omitted Python"
}
if ($pythonManifest -match "r-base") { throw "Python-only manifest contains R" }

# A single-language project must move to both runtimes and back without being
# recreated. Existing packages and tasks in the original runtime stay usable.
& $env:DUAL_BIN enable r --version 4.5
& $env:DUAL_BIN up --refresh
$enabledR = (& $env:DUAL_BIN run scripts/enabled.R) -join "`n"
if (-not ($enabledR -match "R enabled")) { throw "Enabled R runtime did not run" }
$pythonRun = (& $env:DUAL_BIN run pycheck) -join "`n"
if (-not ($pythonRun -match "1.17.0")) { throw "Python task failed after enabling R" }
$bothManifest = Get-Content ".dual/workspace/pyproject.toml" -Raw
if (-not ($bothManifest -match "r-base")) { throw "Mixed manifest omitted R" }
if (-not ($bothManifest -match "(?m)^python = ")) { throw "Mixed manifest omitted Python" }
& $env:DUAL_BIN disable r
& $env:DUAL_BIN up --refresh
$pythonManifest = Get-Content ".dual/workspace/pyproject.toml" -Raw
if ($pythonManifest -match "r-base") { throw "Python-only manifest retained disabled R" }

$rOnly = Join-Path $env:RUNNER_TEMP "dual-r-only"
Remove-Item -Recurse -Force $rOnly -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $rOnly | Out-Null
Set-Location $rOnly
& $env:DUAL_BIN init r-only --r 4.5
& $env:DUAL_BIN add r jsonlite
$config = Get-Content dual.toml -Raw
$rTask = @'
[tasks]
rcheck = 'Rscript -e "cat(jsonlite::toJSON(list(ok=TRUE)))"'
'@
$config = $config.Replace("[tasks]", $rTask.Trim())
Set-Content dual.toml $config
'print("Python enabled")' | Set-Content scripts/enabled.py
$rUp = (& $env:DUAL_BIN --trust-project up) -join "`n"
if (-not ($rUp -match "R 4.5 requested")) {
    throw "R-only preparation did not report R"
}
if ($rUp -match "Python packages configured") {
    throw "R-only preparation reported Python packages"
}
$rRun = (& $env:DUAL_BIN run rcheck) -join "`n"
if (-not ($rRun -match '"ok":\[true\]')) { throw "R-only task failed" }
$rDoctor = (& $env:DUAL_BIN doctor) -join "`n"
if (-not ($rDoctor -match "Python is not required by this environment")) {
    throw "R-only doctor report treated Python as enabled"
}
if (-not (Test-Path dual.lock)) { throw "R-only lockfile was not created" }
$rManifest = Get-Content ".dual/workspace/pyproject.toml" -Raw
if (-not ($rManifest -match "r-base")) { throw "R-only manifest omitted R" }
if ($rManifest -match "(?m)^python = ") { throw "R-only manifest contains Python" }

& $env:DUAL_BIN enable py --version 3.12
& $env:DUAL_BIN up --refresh
$enabledPython = (& $env:DUAL_BIN run scripts/enabled.py) -join "`n"
if (-not ($enabledPython -match "Python enabled")) { throw "Enabled Python runtime did not run" }
$rRun = (& $env:DUAL_BIN run rcheck) -join "`n"
if (-not ($rRun -match '"ok":\[true\]')) { throw "R task failed after enabling Python" }
$bothManifest = Get-Content ".dual/workspace/pyproject.toml" -Raw
if (-not ($bothManifest -match "r-base")) { throw "Mixed manifest omitted R" }
if (-not ($bothManifest -match "(?m)^python = ")) { throw "Mixed manifest omitted Python" }
& $env:DUAL_BIN disable py
& $env:DUAL_BIN up --refresh
$rManifest = Get-Content ".dual/workspace/pyproject.toml" -Raw
if ($rManifest -match "(?m)^python = ") { throw "R-only manifest retained disabled Python" }
Set-Location $mixedRoot

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

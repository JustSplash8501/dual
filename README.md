# dual

**A simple CLI for reproducible projects that use R, Python, or both.**

[![CI](https://github.com/JustSplash8501/dual/actions/workflows/ci.yml/badge.svg)](https://github.com/JustSplash8501/dual/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](#cross-platform-behavior)

You should not need to learn renv, venv, uv, conda, or reticulate just
to run an analysis. `dual` gives a project one user-facing configuration file,
one command-line interface, and one reproducible environment.

> `dual` is an early MVP. The configuration and lock formats
> may change before 1.0.

## Quickstart

```console
dual init cli-tools
dual add r dplyr ggplot2 tidyr
dual add py pandas scikit-learn
dual --trust-project up
dual run analysis
```

Add task commands to `dual.toml` before running them:

```toml
[project]
name = "cli-tools"

[r]
version = "4.5"
cran = [
  "tidyverse",
  "targets@1.11.4",
]
bioc = ["DESeq2"]
github = ["r-lib/pak@v0.9.0"]

[python]
version = "3.12"
dependencies = ["pandas", "scikit-learn", "xgboost"]

[quarto]
enabled = false

[tasks]
analysis = "Rscript scripts/analysis.R"
model = "python scripts/model.py"
report = "quarto render manuscript.qmd"
```

## Philosophy

`dual` is not a package manager. It is a project runner and environment
coordinator. It provides cross-platform R, Python, package resolution,
environments, and lockfiles through one focused interface.

Users edit `dual.toml`, commit `dual.lock`, and run `dual` commands. Internal
environment state is stored under `.dual/` and should not be edited directly.

Python packages are resolved from PyPI. R and Python runtimes are resolved from
conda-forge. Plain R package names such as `dplyr` are also resolved from
conda-forge using the predictable `r-<lowercase-name>` convention.

R packages can explicitly use CRAN, Bioconductor, or GitHub when a conda-forge
build is unavailable:

```toml
[r]
version = "4.5"
packages = [
  "cran::targets@1.11.4",
  "bioc::DESeq2",
  "github::r-lib/pak@v0.9.0",
  "actualName=github::owner/different-repository-name@abc123",
]
```

Source-backed R packages are resolved and installed by
[`pak`](https://pak.r-lib.org/) inside the project environment. Pin GitHub
packages to a tag or commit for reproducibility. When the repository name is
not the R package name, use the `packageName=github::owner/repository` form.
Unlike conda artifacts, old CRAN repository URLs can disappear, so these
source locks are less durable than `dual.lock`. Packages that compile native
code may also require build libraries available from the operating system or
conda-forge.

## Commands

```text
dual init [PROJECT_NAME] [--python VERSION] [--r VERSION]
                                      Create dual.toml and project directories
dual add r PACKAGE...              Add R packages
dual add py PACKAGE...             Add Python packages
dual remove r PACKAGE...           Remove R packages
dual remove py PACKAGE...          Remove Python packages
dual enable r|py [--version VER]   Add or update a project runtime
dual disable r|py [--force]        Remove a project runtime
dual import FILE                   Import pyproject.toml, requirements.txt,
                                   renv.lock, env.lock, uv.lock, or environment.yml
dual up                            Create or update the environment
dual up --refresh                  Re-resolve and update the shared lockfile
dual run TASK                      Run a configured project task
dual run FILE                      Run a .py, .R, .qmd, or .Rmd file
dual sync [--script FILE]          Prepare dependencies without running code
dual deps [--script FILE]          Show effective dependencies
dual export --requirements         Write requirements.txt
dual export --renv                 Write a renv dependency helper
dual export --dockerfile           Write a Dockerfile and .dockerignore
dual task list                     List configured tasks
dual task suggest                  Suggest common tasks from project files
dual shell                         Open a shell in the environment
dual doctor                        Diagnose the project
dual clean [--yes]                 Remove dual-generated environment files
dual lock migrate                  Upgrade dual.lock to the current format
```

Commands including `dual deps`, `dual task list`, `dual task suggest`, `dual
doctor`, and `dual import FILE` accept `--json` for machine-readable output.

Tasks can be simple command strings or dependency-aware tables:

```toml
[tasks]
prepare = "python scripts/prepare.py"
analysis = { cmd = "Rscript scripts/analysis.R", deps = ["prepare"] }
```

When you run `dual run analysis`, Dual runs `prepare` first and rejects
dependency cycles.

Pass script or task arguments after `--`:

```console
dual run analysis.py -- --input data.csv --limit 10
dual run analysis -- --input data.csv --limit 10
```

Existing projects can be brought into Dual with:

```console
dual import pyproject.toml
dual import requirements.txt
dual import renv.lock
dual import uv.lock
dual import environment.yml
dual import env.lock
```

Imports add the dependencies Dual can model today and report skipped entries
such as unsupported conda packages, Python environment markers, direct URL
requirements, local/editable uv packages, or Poetry constraints that do not
map cleanly to PEP 508. PEP 621 dependencies, optional dependency groups,
standard dependency groups, Poetry dependency tables, hashed requirements,
channel-qualified conda packages, and pip entries inside `environment.yml` are
recognized. Pip index directives plus uv and Poetry index tables are imported
into `[[python.index]]`. Requirement hashes are intentionally omitted because
Dual resolves and records its own shared lock. Keep credentials out of index
URLs—Dual rejects embedded URL credentials and preserves PyPI as the primary
index when importing only `--extra-index-url`. Supply private-index
authentication through the invoking environment.

Dual can also discover common test files and suggest task entries without
editing `dual.toml`:

```console
dual task suggest
dual --json task suggest
```

## Script workflows

Dual can keep dependencies next to a Python, R, Quarto, or R Markdown file:

```console
dual init --script analysis.py --python 3.12
dual add --script analysis.py 'requests<3' rich
dual run analysis.py
```

Python uses PEP 723-compatible metadata:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "requests<3",
#   "rich",
# ]
# ///
```

R uses the same block shape with R-specific fields:

```r
# /// script
# r = ">=4.4"
# cran = ["tidyverse", "lme4"]
# bioc = ["DESeq2"]
# github = ["hadley/emo"]
# ///
```

Quarto and R Markdown use an HTML comment:

```markdown
<!-- /// script
python = ">=3.12"
r = ">=4.4"
python-dependencies = ["pandas", "matplotlib"]
cran = ["tidyverse", "knitr"]
bioc = []
github = []
/// -->
```

Use `dual add --script report.qmd --python pandas` or
`dual add --script report.qmd --r tidyverse` when a document can use both
languages. `--index URL`, `--bioc`, and `--github OWNER/REPO` select package
sources. `dual run FILE --dry-run` shows the plan, and `--no-install` requires
an already prepared matching environment.

When a project `dual.toml` is found above the script, Dual merges it with the
inline metadata. Inline version requirements take precedence and dependency
lists are combined without duplicates.

Executable scripts can use this portable shebang on systems whose `env`
supports `-S`:

```text
#!/usr/bin/env -S dual run
```

The shorter `#!/usr/bin/env dual run` form is not portable because many
implementations treat `dual run` as one executable name.

When `PROJECT_NAME` is omitted, `dual init` uses the current directory name.
Project names must start and end with a letter or number and may contain only
ASCII letters, numbers, hyphens, and underscores.

By default, `dual init` creates a mixed R and Python project. Pass one language
option to create a smaller single-language environment, or pass both to select
explicit versions for a mixed project:

```console
dual init python-analysis --python 3.13
dual init r-analysis --r 4.5
dual init mixed-analysis --python 3.13 --r 4.5
```

In `dual.toml`, `[r]` and `[python]` are individually optional. Normal projects
require at least one; a Quarto-only project can instead set `quarto.enabled =
true`. Existing project files containing both sections continue to work
unchanged. Adding or importing a dependency for an omitted language adds that
language with Dual's default runtime version unless the import provides a
version.

Move between single- and mixed-language projects in place:

```console
dual enable r                       # use the default R version
dual enable py --version 3.13       # use an explicit Python version
dual up --refresh

dual disable r
dual up --refresh
```

Enabling an existing runtime without `--version` is a no-op; supplying a new
version updates it without changing its packages. Disabling removes the whole
language section. Dual refuses to disable a runtime that still has configured
packages, Python indexes, or tasks that invoke it or reference compatible
scripts or documents, and it refuses to remove the last runtime unless Quarto
is enabled. `--force` removes configured packages and indexes, but does not
override task or last-runtime protection.
Neither command changes the existing environment or `dual.lock`; run the
suggested `dual up --refresh` to apply the new configuration.

Commands that install packages or execute project code require explicit
repository trust on first use:

```sh
dual --trust-project up
```

Trust is tied to the canonical project path and the contents of all project
files except `.git/`, `.dual/`, `results/`, and Dual's data directory when it
is inside the project. Changing scripts, configuration, lockfiles, data, or
other task inputs requires reviewing and trusting the project again. Generated
files under `results/` do not invalidate trust. Symbolic links and special files
are rejected in trusted projects. CI can set `DUAL_TRUST_PROJECT=1` as an
explicit noninteractive authorization.

## Project environment variables

An optional `.env` at the project root supplies variables to configured tasks,
direct script and document runs, and `dual shell`. The same file works across
Python, R, Quarto, and R Markdown:

```dotenv
DATABASE_URL=postgres://localhost/research
API_TOKEN="local-development-token"
```

```python
import os
print(os.environ["DATABASE_URL"])
```

```r
Sys.getenv("DATABASE_URL")
```

Dual parses `.env` with `dotenvy`, including comments, quotes, multiline values,
escapes, and variable substitution. It reads only `<project-root>/.env` and
never searches parent directories. Values already supplied by the invoking
environment take precedence; within `.env`, the first definition wins.

Project variables are loaded only after trust verification and are attached to
the user-facing child process without modifying Dual's own environment. They do
not affect `dual up`, dependency installation, engine selection, trust, or
diagnostics. In particular, private package-installation credentials must still
be supplied by the invoking environment with `DUAL_ALLOW_CREDENTIALS=1`.

Dual rejects `.env` entries that could alter its control plane, environment
engine, executable lookup, dynamic loader, Python startup path, or R startup
files. This includes `DUAL_*`, `PIXI_*`, `CONDA_*`, `MAMBA_*`, `RATTLER_*`,
`PATH`, home and shell variables, `LD_*`/`DYLD_*` loader variables,
`PYTHONHOME`, `PYTHONPATH`, `PYTHONSTARTUP`, and R home/profile/environment
selectors, library paths, and the reticulate interpreter selector. A symlinked,
malformed, non-UTF-8, or larger-than-1-MiB `.env` is
also rejected. Keep `.env` out of version control by adding it to the project's
`.gitignore`; commit a secret-free `.env.example` when collaborators need a
template.

Treat a Dual project like source code: package installation, lockfile contents,
configured tasks, and interactive shells can execute code with your user
permissions. Dual rejects symbolic links for its configuration and generated
state paths, and it should not be run with elevated privileges.

Environment preparation removes common cloud, registry, and SSH credential
variables before invoking package tooling. Projects that intentionally require
private package credentials can set `DUAL_ALLOW_CREDENTIALS=1` after reviewing
the package sources and build backends.

Pass `--verbose` before or after a command to show additional environment
progress:

```console
dual --verbose up
```

Without `--verbose`, output stays focused on the project.

## Installation

Prebuilt releases install as a single `dual` command. On first use, `dual`
automatically prepares the support files it needs under the user's dual data
directory. It does not modify `PATH` or shell startup files, and users do not
need to install a separate environment tool.

R and Python do not need to be installed globally.

### Install a release

Download the archive for your platform from GitHub Releases, verify it against
`SHA256SUMS`, extract it, and place `dual` (or `dual.exe`) on your `PATH`.
Release archives are produced for Linux x86-64, macOS Apple Silicon, macOS
Intel, and Windows x86-64. GitHub build-provenance attestations are published
for every archive.

Release signing is enabled when maintainers configure the Apple and Windows
signing secrets documented in `CONTRIBUTING.md`. Without those optional
credentials, releases still include SHA-256 checksums and GitHub provenance
attestations.

### Build from source

[Rust](https://rustup.rs) 1.88 or newer is required only when building from
source.

```console
git clone https://github.com/JustSplash8501/dual.git
cd dual
cargo build --release
```

The executable is written to `target/release/dual` on Linux and macOS, or
`target\release\dual.exe` on Windows. Put it somewhere on your `PATH`.

During development:

```console
cargo run -- --help
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

### Publishing releases

Pushing a version tag that matches `Cargo.toml` builds archives for Linux
x86-64, macOS Apple Silicon, macOS Intel, and Windows x86-64. The release
workflow publishes a consolidated `SHA256SUMS` file and GitHub provenance
attestations:

```console
git tag v0.1.1
git push origin v0.1.1
```

## Mixed R/Python example

[`examples/basic-mixed`](examples/basic-mixed) contains an R task, a Python
task, and a `dual.toml` that installs both languages. Try it after building:

```console
cd examples/basic-mixed
../../target/release/dual --trust-project up
../../target/release/dual doctor
../../target/release/dual run analysis
../../target/release/dual run model
```

When `reticulate` is listed as an R package, `dual up` verifies that it can use
the Python interpreter from the project environment.

## Generated files

`dual up`, `dual sync`, and successful script preparation create:

- `dual.lock` — the exact, shareable resolution for conda-forge, PyPI, CRAN,
  Bioconductor, and GitHub dependencies
- `.dual/` — local generated environment state

`.dual/` is generated locally and ignored. `dual.lock` is intentionally
committed. It is a Dual-owned lockfile containing a neutral `environment`
resolution, source-backed R resolution when needed, and a stable metadata
summary containing requested runtime versions, direct dependencies, package
sources, and an update timestamp. Internal generated formats remain
implementation details under `.dual/`.

When a collaborator receives `dual.toml` and `dual.lock`, `dual up` creates the
environment with the shared resolution enforced.
If `dual.toml` is intentionally changed, run `dual up --refresh` to re-resolve
dependencies. Commit the updated `dual.toml` and `dual.lock` together.

Environment updates are failure-safe. Dual does not mark a new environment as
ready until dependency installation, source-backed R packages, bridge setup,
and runtime validation all succeed. If an update fails, it restores the prior
generated manifest, shared lockfile, readiness marker, and R/Python bridge. A
failed first preparation removes its incomplete generated state; downloaded
package caches may remain available for the next attempt.

`dual clean` removes only `.dual/`. It deliberately preserves `dual.lock`,
`dual.toml`, scripts, data, results, and other user files.

## Compatibility contract

The following files and behaviors are public contracts:

- `dual.toml` is strict TOML. Documented fields, legacy R and Python `packages`
  aliases, optional `[r]`/`[python]` sections, and string or detailed task
  forms are supported. Unknown fields are rejected so misspellings cannot
  silently change an environment.
- `dual.lock` is a Dual-owned, versioned JSON file. Lock format version 1 and
  its legacy `pixi` field spelling remain readable; `dual lock migrate` rewrites
  the legacy spelling. The `environment` payload is opaque and must not be
  edited by hand. A newer unsupported lock version fails safely instead of
  being guessed.
- Inline script metadata is strict TOML inside the documented comment markers.
  Python uses the PEP 723 fields Dual supports; R and mixed documents use the
  documented Dual extensions. Unknown fields and cross-language fields in a
  single-language script are rejected.
- Project-root `.env` is local execution input, not dependency configuration.
  Its values never enter `dual.lock`, generated manifests, dependency exports,
  or Docker build contexts. Commit `.env.example`, not `.env`.
- Plain `dual up` and project `dual sync` enforce an existing `dual.lock` and
  create one when absent. Only `dual up --refresh` intentionally re-resolves a
  project and updates the shared lock. Script sync prepares the script-specific
  effective environment.

Commit `dual.toml` and `dual.lock` together after an intentional refresh. Keep
`.dual/` and `.env` local.

## Docker export

`dual export --dockerfile` rewrites the generated `Dockerfile` but preserves
existing `.dockerignore` content and appends any missing safety rules. It also
excludes `.env`, `.env.*`, `.dual/`, Git metadata, Rust build output, and
`results/`; `.env.example` remains available to the build context.

Python-only exports use the configured Python version as the official Python
image tag. R and mixed exports use the configured `rocker/r-ver` tag. In a
mixed export, the R image's distribution supplies Python; the build verifies
that its major/minor series matches `python.version` and fails with a direct
explanation if it does not. Configured Python indexes are written to both
`requirements.txt` and the Docker installation input.

Docker image selection needs an exact version or a usable lower bound such as
`>=3.12`. Wildcards, upper-bound-only constraints, and strict greater-than
constraints are rejected because they do not identify a safe base-image tag.

R packages are installed through `pak`, so CRAN, Bioconductor, GitHub, aliases,
and supported version pins keep their `dual.toml` meaning. Native R and Python
packages can require operating-system development libraries. Supply them
without rewriting the generated install layer:

```console
docker build \
  --build-arg DUAL_SYSTEM_PACKAGES="libcurl4-openssl-dev libssl-dev libxml2-dev" \
  .
```

The generated image intentionally does not infer system libraries, install
Quarto, copy `.env`, reproduce task execution, or replace a reviewed production
container design. CI smoke testing builds a mixed R/Python export for pull
requests and weekly with CRAN and PyPI packages plus a real system-library
dependency.

## Cross-platform behavior

The CLI targets Linux, macOS Intel, macOS Apple Silicon, and Windows 10/11.
Generated environments declare `linux-64`, `osx-64`, `osx-arm64`, and
`win-64`. Commands run through the project environment instead of assuming a
global R, Python, shell, or `.venv` layout. `dual shell` opens an activated
shell whose prompt is prefixed with the project name, such as `(cli-tools)`.
Interactive R sessions also identify the loaded project and Dual version:

```text
R 4.6.0 restarted.
- Project '~/path/to/cli-tools' loaded. [dual 0.1.1]
```

The R version line is produced by the editor from the actual configured
interpreter; Dual produces only the project-loaded line. Dual preserves the
usual R startup behavior by loading the project's `.Rprofile`, or the user's
`~/.Rprofile` when the project does not provide one, before printing its
banner. This allows tools such as `renv` to continue activating normally.

## Scope

The MVP deliberately has no GUI, editor integration, or SLURM support. Quarto
and R Markdown files can be run directly. Docker export remains a reviewable
starting point rather than a complete container build system. The goal is a
small, legible foundation that makes ordinary scientific projects easy to
reproduce.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for development setup, required
checks, real environment integration tests, and pull request guidance.

## License

MIT

See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for automatically
provisioned third-party components.

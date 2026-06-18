# dual

**A simple CLI for reproducible projects that use R, Python, or both.**

[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![R](https://img.shields.io/badge/R-276DC3?style=for-the-badge&logo=r&logoColor=white)](https://www.r-project.org/)
[![Python](https://img.shields.io/badge/Python-3776AB?style=for-the-badge&logo=python&logoColor=white)](https://www.python.org/)
[![ChatGPT](https://img.shields.io/badge/ChatGPT-74aa9c?style=for-the-badge&logo=openai&logoColor=white)](https://chatgpt.com/)
[![Cross-platform](https://img.shields.io/badge/Cross--platform-Linux%20%7C%20macOS%20%7C%20Windows-6C63FF?style=for-the-badge)](#cross-platform-behavior)
[![CI](https://img.shields.io/badge/CI-GitHub_Actions-2088FF?style=for-the-badge&logo=githubactions&logoColor=white)](.github/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow?style=for-the-badge)](LICENSE)

You should not need to learn renv, venv, uv, conda, or reticulate just
to run an analysis. `dual` gives a project one user-facing configuration file,
one command-line interface, and one reproducible environment.

> `dual` is an early MVP. The configuration and lock formats
> may change before 1.0.

## Quickstart

```console
dual init
dual add r tidyverse lme4 emmeans
dual add py pandas scikit-learn
dual up
dual run analysis
```

Add task commands to `dual.toml` before running them:

```toml
[project]
name = "beef-analysis"

[r]
version = "4.5"
packages = [
  "tidyverse",
  "cran::targets@1.11.4",
  "bioc::DESeq2",
  "github::r-lib/pak@v0.9.0",
]

[python]
version = "3.12"
packages = ["pandas", "scikit-learn", "xgboost"]

[tasks]
analysis = "Rscript scripts/analysis.R"
model = "python scripts/model.py"
report = "quarto render manuscript.qmd"
```

## Philosophy

`dual` is not a package manager. It is a project runner and environment
coordinator built on a proven environment engine. It provides cross-platform
R, Python, conda-forge, PyPI, environments, and lockfiles through one focused
interface.

Users edit `dual.toml`, commit `dual.lock`, and run `dual` commands. Internal
environment state is stored under `.dual/` and should not be edited directly.

Python packages are resolved from PyPI. R and Python runtimes are resolved from
conda-forge. Plain R package names such as `tidyverse` are also resolved from
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
dual init [--force] [--name NAME]  Create dual.toml and project directories
dual add r PACKAGE...              Add R packages
dual add py PACKAGE...             Add Python packages
dual up                            Create or update the environment
dual up --refresh                  Re-resolve and update the shared lockfile
dual run TASK                      Run a configured task
dual shell                         Open a shell in the environment
dual doctor                        Diagnose the project
dual clean [--yes]                 Remove dual-generated environment files
```

Pass `--verbose` before or after a command to show additional environment
progress:

```console
dual --verbose up
```

Without `--verbose`, output stays focused on the project.

## Installation

Prebuilt releases install as a single `dual` command. On first use, `dual`
automatically installs private environment support under the user's dual data
directory. It does not modify `PATH` or shell startup files, and users do not
need to install a separate environment tool.

R and Python do not need to be installed globally.

### Install a release

Download the archive for your platform from GitHub Releases, verify it against
`SHA256SUMS`, extract it, and place `dual` (or `dual.exe`) on your `PATH`.
Release archives are produced for Linux x86-64, macOS Apple Silicon, macOS
Intel, and Windows x86-64. GitHub build-provenance attestations are published
for every archive.

### Build from source

[Rust](https://rustup.rs) 1.80 or newer is required only when building from
source.

```console
git clone https://github.com/OWNER/dual.git
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
git tag v0.1.0
git push origin v0.1.0
```

## Mixed R/Python example

[`examples/basic-mixed`](examples/basic-mixed) contains an R task, a Python
task, and a `dual.toml` that installs both languages. Try it after building:

```console
cd examples/basic-mixed
../../target/release/dual up
../../target/release/dual doctor
../../target/release/dual run analysis
../../target/release/dual run model
```

When `reticulate` is listed as an R package, `dual up` verifies that it can use
the Python interpreter from the project environment.

## Generated files

`dual up` creates:

- `dual.lock` — the exact, shareable resolution for conda-forge, PyPI, CRAN,
  Bioconductor, and GitHub dependencies
- `.dual/` — local generated environment state

`.dual/` is generated locally and ignored. `dual.lock` is intentionally
committed. It is a Dual-owned lockfile containing a neutral `environment`
resolution and, when needed, the source-backed R resolution. Internal engine
formats remain private implementation details under `.dual/`.

When a collaborator receives `dual.toml` and `dual.lock`, `dual up` creates the
environment with the shared resolution enforced.
If `dual.toml` is intentionally changed, run `dual up --refresh` to re-resolve
dependencies. Commit the updated `dual.toml` and `dual.lock` together.

`dual clean` removes only `.dual/`. It deliberately preserves `dual.lock`,
`dual.toml`, scripts, data, results, and other user files.

## Cross-platform behavior

The CLI targets Linux, macOS Intel, macOS Apple Silicon, and Windows 10/11.
Generated environments declare `linux-64`, `osx-64`, `osx-arm64`, and
`win-64`. Commands run through the project environment instead of assuming a
global R, Python, shell, or `.venv` layout. On Unix, `dual shell` uses the
user's configured shell; on Windows it uses the configured command processor
or PowerShell.

## Scope

The MVP deliberately has no GUI, editor integration, Docker support, SLURM
support, or special Quarto behavior. Quarto works like any other task command.
The goal is a small, legible foundation that makes ordinary scientific
projects easy to reproduce.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for development setup, required
checks, real environment integration tests, and pull request guidance.

## License

MIT

See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for automatically
provisioned third-party components.

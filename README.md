<p align="center">
  <img src="logo/dual_repo_logo_avatar.png" alt="dual logo" width="120">
</p>

# dual

**A cross-platform CLI for reproducible scientific projects using R, Python, or both—one config, one lockfile, one command.**

[![CI](https://github.com/JustSplash8501/dual/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/JustSplash8501/dual/actions/workflows/ci.yml?query=branch%3Amain)
[![Release](https://img.shields.io/github/v/release/JustSplash8501/dual)](https://github.com/JustSplash8501/dual/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](#supported-platforms)

Scientific projects often accumulate separate tools for R, Python, runtime
versions, package installation, task execution, and lockfiles. Dual coordinates
those pieces behind one project-level interface:

- declare R and Python dependencies together in `dual.toml`
- commit one `dual.lock` so collaborators use the same resolution
- run scripts and named tasks inside the project environment
- work across Linux, macOS, and Windows without globally installing R or Python

Dual is an early pre-1.0 project. Configuration and lock formats may evolve as
the project gathers real-world feedback.

## See it in action

<p align="center">
  <img src="docs/assets/dual-demo.gif" alt="Creating a mixed R and Python project and adding dependencies with Dual" width="800">
</p>

## Install

Download the archive for your platform from the
[latest GitHub release](https://github.com/JustSplash8501/dual/releases/latest),
verify it against `SHA256SUMS`, extract it, and place `dual` (or `dual.exe`) on
your `PATH`.

Prebuilt archives are available for:

- Linux x86-64
- macOS Apple Silicon
- macOS Intel
- Windows x86-64

The release archives include SHA-256 checksums and GitHub build-provenance
attestations. Dual installs its private environment support on first use. It
does not modify shell startup files, and neither R nor Python needs to be
installed globally.

To build from source instead, install Rust 1.88 or newer and run:

```console
git clone https://github.com/JustSplash8501/dual.git
cd dual
cargo build --locked --release
```

The binary will be at `target/release/dual` on Linux and macOS or
`target\release\dual.exe` on Windows.

## Try the mixed-language example

The repository includes a small project that runs one R analysis and one
Python model in the same managed environment:

```console
git clone https://github.com/JustSplash8501/dual.git
cd dual/examples/basic-mixed
dual --trust-project up
dual run analysis
dual run model
```

The example creates `results/languages.csv` and `results/model.txt`. It also
checks that R's `reticulate` can use the Python interpreter from the project
environment.

`--trust-project` is intentional: package installation and project tasks can
execute code with your user permissions. Review an unfamiliar project before
trusting it.

## Start a project

Create a directory, initialize Dual, and add dependencies from both ecosystems:

```console
mkdir cli-tools
cd cli-tools

dual init --r 4.5 --python 3.12
dual add r dplyr ggplot2 tidyr
dual add py pandas scikit-learn
dual --trust-project up
```

Dual writes the human-edited project definition to `dual.toml`. Named tasks can
be strings or dependency-aware tables:

```toml
[project]
name = "cli-tools"

[r]
version = "4.5"
cran = ["dplyr", "ggplot2", "tidyr"]
bioc = []
github = []

[python]
version = "3.12"
dependencies = ["pandas", "scikit-learn"]

[tasks]
prepare = "python scripts/prepare.py"
analysis = { cmd = "Rscript scripts/analysis.R", deps = ["prepare"] }
```

Run the workflow with:

```console
dual run analysis
```

Dual runs `prepare` first, rejects dependency cycles, and forwards arguments
placed after `--` to the requested script or task.

## Command overview

```text
dual init                   Initialize a project or an inline script
dual add                    Add R or Python dependencies
dual remove                 Remove R or Python dependencies
dual enable                 Add or update a project runtime
dual disable                Remove a project runtime
dual import                 Import an existing dependency or lock file
dual up                     Create or update the project environment
dual run                    Run a named task or supported file
dual sync                   Prepare dependencies without running code
dual deps                   Show the effective dependencies
dual export                 Write requirements, renv, or Docker helpers
dual task list              List configured tasks
dual task suggest           Suggest tasks found in project files
dual lock migrate           Upgrade dual.lock to the current format
dual cache dir              Print the shared cache directory
dual cache info             Show shared cache usage
dual cache prune            Remove obsolete cache layouts
dual cache clean            Remove all shared cache entries
dual shell                  Open a shell in the project environment
dual doctor                 Diagnose Dual and the current project
dual clean                  Remove generated project environment files
```

Run `dual <command> --help` for options and examples, or see the
[complete command reference](docs/reference.md#command-reference).

## What Dual manages

| File or directory | Purpose | Commit it? |
| --- | --- | --- |
| `dual.toml` | Runtimes, direct dependencies, sources, and tasks | Yes |
| `dual.lock` | Exact shared dependency resolution | Yes |
| `.dual/` | Generated local project environment | No |
| `.env` | Optional local variables supplied to trusted tasks | No |

Plain `dual up` enforces an existing lockfile and creates one when none exists.
Use `dual up --refresh` only when you intentionally want to resolve new package
versions, then commit `dual.toml` and `dual.lock` together.

Downloaded packages are cached across projects while each project's environment
remains isolated under `.dual/`.

## Designed for mixed scientific workflows

Dual can:

- create mixed, Python-only, R-only, and Quarto-enabled projects
- resolve Python packages from PyPI and R packages from conda-forge, CRAN,
  Bioconductor, or GitHub
- import `pyproject.toml`, `requirements.txt`, `uv.lock`, `renv.lock`,
  `environment.yml`, and `env.lock`
- run Python, R, Quarto, and R Markdown files directly
- keep dependencies beside individual scripts using PEP 723-compatible metadata
  for Python and equivalent metadata blocks for R and mixed documents
- define task dependencies and forward command-line arguments
- export `requirements.txt`, an renv dependency helper, or a reviewable Dockerfile
- produce machine-readable JSON for inspection and automation commands

For example, a standalone Python script can carry its own environment request:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "requests<3",
#   "rich",
# ]
# ///
```

Run it with:

```console
dual run analysis.py
```

See the [reference manual](docs/reference.md) for R, Quarto, and R Markdown
metadata formats and the complete command reference.

## Why another tool?

`renv`, `uv`, virtual environments, conda, and reticulate each solve important
parts of this problem. Dual is useful when the project boundary crosses those
ecosystems and collaborators should not need to assemble the same toolchain by
hand.

Dual is not a replacement package manager. It is a project runner and
environment coordinator that presents package resolution, runtimes, lockfiles,
and tasks through one focused interface.

## Safety and reproducibility

Dual treats project configuration, dependency installation, and task execution
as trust boundaries. Among other safeguards, it:

- requires explicit trust before installing packages or executing project code
- invalidates trust when relevant project inputs change
- rejects unsafe symbolic links and special files in managed paths
- strips common cloud, registry, and SSH credentials from package operations by
  default
- applies project `.env` values only to trusted user tasks and shells, not to
  Dual's own control plane
- updates environments and lockfiles failure-safely
- serializes concurrent commands that could modify the same project

Do not run Dual with elevated privileges. See [SECURITY.md](SECURITY.md) for the
vulnerability-reporting policy and the [reference manual](docs/reference.md)
for the detailed safety contract.

## Supported platforms

Dual targets:

- Linux x86-64 (`linux-64`)
- macOS Intel (`osx-64`)
- macOS Apple Silicon (`osx-arm64`)
- Windows 10/11 x86-64 (`win-64`)

Commands run through the project environment instead of assuming a global R,
Python, shell, or `.venv` layout.

## Scope

Dual currently has no GUI, editor integration, or SLURM support. Quarto and R
Markdown files can be run directly. Docker export is a reviewable starting point
rather than a complete production container system.

## Documentation and contributing

- [Reference manual](docs/reference.md)
- [Mixed R/Python example](examples/basic-mixed)
- [Contributing guide](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Release downloads](https://github.com/JustSplash8501/dual/releases)

Issues and pull requests are welcome. For substantial behavior or configuration
changes, please open an issue first so the design can be discussed.

## License

Dual is available under the [MIT License](LICENSE). See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for automatically provisioned
third-party components.

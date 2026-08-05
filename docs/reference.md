# dual Reference Manual

**Package:** dual  
**Version:** 0.1.1  
**Title:** Reproducible R and Python Projects Without Environment-Tool Busywork  
**Description:** `dual` is a command-line project runner and environment coordinator for projects that use R, Python, Quarto, R Markdown, or combinations of these tools. It uses a single `dual.toml` project file, optional inline script metadata, a shared `dual.lock`, and private generated state under `.dual/`.

This manual follows the style of R package documentation: each entry has a name, usage, arguments, value, details, examples, and related entries where useful.

## Package Overview

### Name

`dual-package`

### Usage

```console
dual <COMMAND> [OPTIONS]
```

### Description

`dual` coordinates reproducible computational environments for projects and scripts. It manages R packages, Python packages, runtime versions, configured project tasks, inline script metadata, imports from existing dependency files, export helpers, diagnostics, and trust checks.

### Details

The main project file is `dual.toml`. Generated environment state is stored in `.dual/`; users should not edit that directory directly. The public lockfile is `dual.lock`.

Commands that install packages or execute project code require trust. Trust can be granted with `--trust-project` after reviewing the project, or noninteractively in CI with `DUAL_TRUST_PROJECT=1`.

### See Also

`dual-init`, `dual-add`, `dual-up`, `dual-run`, `dual-sync`, `dual-deps`, `dual-doctor`

## Project Configuration

### Name

`dual.toml`

### Usage

```toml
[project]
name = "my-project"

[r]
version = "4.5"
cran = []
bioc = []
github = []

[python]
version = "3.12"
dependencies = []

[quarto]
enabled = false

[tasks]
analysis = "Rscript scripts/analysis.R"
model = { cmd = "python scripts/model.py", deps = ["analysis"] }
```

### Fields

`project.name`: A project identifier used in configuration and shell prompts. It must start and end with an ASCII letter or number and may contain ASCII letters, numbers, hyphens, and underscores.

`r.version`: R runtime version requirement.

`r.cran`: CRAN package names. Plain R package names are serialized here.

`r.bioc`: Bioconductor package names.

`r.github`: GitHub package references in `OWNER/REPO`, `OWNER/REPO@REF`, or `PackageName=OWNER/REPO@REF` form.

`python.version`: Python runtime version requirement.

`python.dependencies`: Python package requirements such as `pandas`, `requests<3`, or `scikit-learn==1.6.1`.

`python.index`: Optional package indexes, written as an array of tables with `url` fields.

`quarto.enabled`: Whether Quarto support should be included.

`tasks`: Named commands, either as strings or as tables with `cmd` and optional `deps`.

### Details

The `[r]` and `[python]` sections are individually optional. Normal projects
must contain at least one; a Quarto-only project can omit both when
`quarto.enabled` is true. Omitting a language section prevents that runtime and
its bridge settings from being installed. Existing mixed-language
configurations containing both sections remain valid.

When `dual add` or `dual import` introduces a dependency for an omitted
language, Dual adds the missing section. It uses R 4.5 or Python 3.12 by default
unless an imported file specifies a runtime version.

`dual enable` and `dual disable` explicitly manage the same optional sections.
They update `dual.toml` but leave the generated environment and `dual.lock`
untouched until `dual up --refresh` is run.

R packages are normalized internally as source-qualified references such as `cran::dplyr`, `bioc::DESeq2`, and `github::r-lib/pak@v0.9.0`. Python dependencies are validated as simple package requirements and direct URLs or environment markers are rejected.

Task dependencies are resolved before execution. Dependency cycles are rejected.

### Examples

```toml
[tasks]
prepare = "python scripts/prepare.py"
analysis = { cmd = "Rscript scripts/analysis.R", deps = ["prepare"] }
report = "quarto render report.qmd"
```

## Project Environment

### Name

`dual-project-environment`

### Usage

```dotenv
# <project-root>/.env
DATABASE_URL=postgres://localhost/research
API_TOKEN="local-development-token"
```

### Details

Dual parses only the `.env` file at the resolved project root. It does not
search ancestor directories and does not modify Dual's process environment.
Parsed values are attached to configured tasks, direct Python/R/Quarto/R
Markdown runs, and `dual shell` after project trust is established. Python can
read them with `os.environ`; R can read them with `Sys.getenv()`.

The invoking process environment takes precedence over `.env`. Within the
file, the first definition of a name wins. `dotenvy` syntax supports comments,
quotes, multiline values, escapes, and variable substitution.

`.env` is not loaded by environment creation, dependency installation,
diagnostics, inspection, or trust evaluation. Reserved control-plane and
process-injection variables are rejected, including `DUAL_*`, environment
engine prefixes, executable/home/shell variables, dynamic-loader variables,
Python path/startup selectors, and R home/profile/environment selectors. The
file must be UTF-8, no larger than 1 MiB, and not a symbolic link. Its contents
participate in the project trust fingerprint and are never printed or written
to `dual.lock`.

`.env` uses dotenv syntax and does not replace R's separate `.Renviron` startup
format.

## Inline Script Metadata

### Name

`script-metadata`

### Usage

```console
dual init --script analysis.py --python 3.12
dual add --script analysis.py requests rich
dual run analysis.py
dual sync --script analysis.py
dual deps --script analysis.py
```

### Details

Python and R scripts use comment-delimited TOML blocks. Quarto and R Markdown documents use HTML comments.

Python files support PEP 723-compatible metadata:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "requests<3",
#   "rich",
# ]
# ///
```

R files use R-specific fields:

```r
# /// script
# r = ">=4.4"
# cran = ["tidyverse"]
# bioc = ["DESeq2"]
# github = ["hadley/emo"]
# ///
```

Quarto and R Markdown can combine Python and R metadata:

```markdown
<!-- /// script
python = ">=3.12"
r = ">=4.4"
python-dependencies = ["pandas", "matplotlib"]
cran = ["knitr"]
bioc = []
github = []
/// -->
```

When a script lives below a project containing `dual.toml`, the project configuration and inline metadata are merged. Inline runtime version requirements take precedence, and dependency lists are deduplicated.

## Command Reference

### `dual init`

#### Name

`dual-init`

#### Usage

```console
dual init [PROJECT_NAME] [--python VERSION] [--r VERSION] [--force]
dual init --script FILE [--python VERSION] [--r VERSION] [--force]
```

#### Arguments

`PROJECT_NAME`: Optional project name. If omitted, the current directory name is used.

`--force`: Replace an existing `dual.toml` or inline metadata block.

`--script FILE`: Create or update inline metadata in a `.py`, `.R`, `.qmd`, or `.Rmd` file.

`--python VERSION`: Include Python at this version. When used alone for project initialization, omit R.

`--r VERSION`: Include R at this version. When used alone for project initialization, omit Python.

#### Value

Writes `dual.toml` for projects, or inserts an inline metadata block for scripts. With no language options, project initialization remains mixed-language. Existing scripts are preserved except for the inserted or replaced metadata block.

#### Examples

```console
dual init cli-tools
dual init python-analysis --python 3.13
dual init r-analysis --r 4.5
dual init --script scripts/analysis.R --r 4.5
dual init --script report.qmd --python 3.12 --r 4.5
```

### `dual add`

#### Name

`dual-add`

#### Usage

```console
dual add r PACKAGE...
dual add py PACKAGE...
dual add --script FILE [--python | --r] [--index URL] [--bioc] [--github OWNER/REPO] PACKAGE...
```

#### Arguments

`r`, `py`: Project dependency ecosystem.

`PACKAGE`: One or more package requirements or names.

`--script FILE`: Add dependencies to inline script metadata.

`--python`, `--r`: Select a language when adding to a mixed-language document.

`--index URL`: Add a Python package index.

`--bioc`: Add R packages as Bioconductor dependencies.

`--github OWNER/REPO`: Add an R GitHub source dependency.

#### Value

Updates `dual.toml` or the inline metadata block. Existing entries are deduplicated.

#### Examples

```console
dual add r dplyr ggplot2
dual add r bioc::DESeq2 github::r-lib/pak@v0.9.0
dual add py pandas 'requests<3'
dual add --script report.qmd --python pandas
dual add --script report.qmd --github hadley/emo
```

### `dual remove`

#### Name

`dual-remove`

#### Usage

```console
dual remove r PACKAGE...
dual remove py PACKAGE...
```

#### Arguments

`r`, `py`: Dependency ecosystem to edit.

`PACKAGE`: One or more package names or canonical references.

#### Value

Removes matching entries from `dual.toml` and reports how many were removed.

#### Examples

```console
dual remove py pandas
dual remove r bioc::DESeq2
```

### `dual enable`

#### Name

`dual-enable`

#### Usage

```console
dual enable r [--version VERSION]
dual enable py [--version VERSION]
```

#### Arguments

`r`, `py`: Runtime to enable. `python` is accepted as an alias for `py`.

`--version VERSION`: Runtime version to request. An omitted runtime defaults to
R 4.5 or Python 3.12. For an enabled runtime, omitting this option preserves the
current version.

#### Value

Adds the missing runtime section or updates its version. Existing packages,
indexes, tasks, generated state, and `dual.lock` are preserved. Repeating the
same request is a no-op.

#### Examples

```console
dual enable r
dual enable py --version 3.13
dual up --refresh
```

### `dual disable`

#### Name

`dual-disable`

#### Usage

```console
dual disable r [--force]
dual disable py [--force]
```

#### Arguments

`r`, `py`: Runtime to disable. `python` is accepted as an alias for `py`.

`--force`: Remove packages and Python indexes configured within the disabled
runtime section.

#### Value

Removes the runtime section. Without `--force`, configured packages or indexes
block the operation. Tasks that invoke the runtime or reference compatible
scripts always block it; `.qmd` task targets conservatively block either runtime
because they can execute both. Removing the last runtime from a project without
Quarto enabled is also blocked. An already omitted runtime is a no-op. Generated
state and `dual.lock` remain unchanged until `dual up --refresh` applies the
transition.

#### Examples

```console
dual disable r
dual disable py --force
dual up --refresh
```

### `dual import`

#### Name

`dual-import`

#### Usage

```console
dual import FILE
dual --json import FILE
```

#### Arguments

`FILE`: A dependency source. Supported inputs are `pyproject.toml`, `requirements.txt`, `renv.lock`, `uv.lock`, `environment.yml`, `environment.yaml`, and generic `env.lock` files.

#### Value

Updates `dual.toml` with supported dependencies and prints an import report. With `--json`, prints the report as JSON.

#### Details

Unsupported entries, such as direct URL Python requirements, Python environment markers, Poetry constraints that do not map cleanly to PEP 508, or unmodeled conda packages, are skipped and reported.

#### Examples

```console
dual import pyproject.toml
dual import requirements.txt
dual import renv.lock
dual --json import environment.yml
```

### `dual up`

#### Name

`dual-up`

#### Usage

```console
dual --trust-project up
dual --trust-project up --refresh
```

#### Arguments

`--refresh`: Re-resolve dependencies and update `dual.lock`.

#### Value

Creates or updates the project environment, validates it, and refreshes project trust when successful.

#### Details

The environment engine is installed automatically when needed. Package tooling runs with common credential environment variables removed unless `DUAL_ALLOW_CREDENTIALS=1` is set.

Updates become ready only after dependency installation, source-backed R
package installation, R/Python bridge preparation, and runtime validation all
succeed. On failure, Dual restores the previous generated manifest,
`dual.lock`, readiness marker, and bridge. If no prior ready environment
existed, incomplete generated state is removed. The attempted `dual.toml`
change is not reverted; correct it or restore its previous contents before
using the preserved environment.

### `dual run`

#### Name

`dual-run`

#### Usage

```console
dual --trust-project run TASK [-- ARG...]
dual --trust-project run FILE [--no-install] [--dry-run] [-- ARG...]
```

#### Arguments

`TASK`: A name from `[tasks]`.

`FILE`: A `.py`, `.R`, `.qmd`, or `.Rmd` file.

`ARG`: Arguments passed to the selected task or script after `--`.

`--no-install`: Require an already prepared matching script environment.

`--dry-run`: Print the planned preparation and command without changing files or running code.

#### Value

Runs dependency tasks first for named project tasks, then runs the requested task. For script files, prepares the script environment unless `--no-install` is used, then runs the generated script task.

After trust verification, values from the project-root `.env` are supplied to
the executed task or script. They are not used while preparing dependencies.

#### Examples

```console
dual --trust-project run analysis
dual --trust-project run analysis -- --input data.csv
dual --trust-project run scripts/model.py -- --seed 1
dual run report.qmd --dry-run
```

### `dual sync`

#### Name

`dual-sync`

#### Usage

```console
dual --trust-project sync
dual --trust-project sync --script FILE
dual sync [--script FILE] --dry-run
```

#### Arguments

`--script FILE`: Prepare dependencies for a script or document instead of the project.

`--dry-run`: Print the plan without changing files.

#### Value

Prepares dependencies without running project code.

### `dual deps`

#### Name

`dual-deps`

#### Usage

```console
dual deps
dual deps --script FILE
dual --json deps
dual --json deps --script FILE
```

#### Arguments

`--script FILE`: Show effective dependencies for a script or document.

#### Value

Prints effective Python and R dependencies. With `--json`, prints a structured dependency report.

### `dual export`

#### Name

`dual-export`

#### Usage

```console
dual export --requirements
dual export --renv
dual export --dockerfile
```

#### Arguments

`--requirements`: Write `requirements.txt`.

`--renv`: Write `renv-dependencies.R`.

`--dockerfile`: Write `Dockerfile` and `.dockerignore`.

#### Value

Returns the path of the written file and writes the selected export artifact.

#### Details

Exports are compatibility helpers and should be reviewed before production use.

### `dual task list`

#### Name

`dual-task-list`

#### Usage

```console
dual task list
dual --json task list
```

#### Value

Prints configured tasks. With `--json`, prints an array of task records with `name`, `command`, and `deps`.

### `dual task suggest`

#### Name

`dual-task-suggest`

#### Usage

```console
dual task suggest
dual --json task suggest
```

#### Value

Discovers common Python and R test files and suggests task names, commands, and packages. With `--json`, prints the discovered files and suggestions.

### `dual shell`

#### Name

`dual-shell`

#### Usage

```console
dual --trust-project shell
```

#### Value

Opens an interactive shell inside the project environment.

### `dual doctor`

#### Name

`dual-doctor`

#### Usage

```console
dual doctor
dual --json doctor
```

#### Value

Checks local system tools, project configuration, environment support, lockfile state, runtime availability, package installation, R/Python bridge status, and configured tasks. With `--json`, prints a structured diagnostic report.

### `dual clean`

#### Name

`dual-clean`

#### Usage

```console
dual clean
dual clean --yes
```

#### Arguments

`--yes`, `-y`: Skip the confirmation prompt.

#### Value

Removes files and environments generated by `dual`, primarily `.dual/` managed state.

### `dual engine`

#### Name

`dual-engine`

#### Usage

```console
dual engine update
dual engine uninstall
```

#### Value

`update` downloads and activates the pinned environment engine. `uninstall` removes the private engine installation and reports whether anything was removed.

### `dual lock`

#### Name

`dual-lock`

#### Usage

```console
dual lock migrate
```

#### Value

Rewrites `dual.lock` using the current lockfile format and reports whether migration occurred.

## Rust Library Reference

The crate exposes modules for use by the CLI and tests. These APIs are application-facing and may change before 1.0.

### Configuration API

#### Name

`dual::config`

#### Usage

```rust
use dual::config::{Config, TaskConfig};

let root = Config::find_root(std::path::Path::new("."))?;
let config = Config::load(&root)?;
```

#### Main Types

`Config`: Typed `dual.toml` model with `project`, optional `r` and `python` languages, `quarto`, and `tasks`.

`ProjectConfig`: Project metadata.

`RConfig`: R runtime and package references.

`PythonConfig`: Python runtime, dependencies, and indexes.

`PackageIndex`: Python package index URL.

`QuartoConfig`: Quarto enablement.

`TaskConfig`: Either `Command(String)` or `Detailed(TaskDetails)`.

`TaskDetails`: Command plus task dependencies.

`EffectiveConfig`: Merged project and inline script configuration.

`EnableLanguageResult`: Whether enablement changed the file, plus the previous
and selected runtime versions.

`DisableLanguageResult`: Whether disablement changed the file, plus removed
package and index counts.

`MetadataSource`: Source label for effective script configuration.

`PythonRequirement`: Parsed Python requirement with `name`, `extras`, and `version`.

#### Main Functions

`Config::path(root)`: Return `root/dual.toml`.

`Config::load(root)`: Load and validate `dual.toml`.

`Config::find_root(start)`: Search ancestors for `dual.toml`.

`Config::find_root_optional(start)`: Optional ancestor search.

`Config::for_script(path)`: Build an effective script configuration.

`Config::empty(project_name)`: Create a disabled empty configuration.

`Config::merge_script_metadata(metadata)`: Merge inline metadata into a config.

`Config::from_path(path)`: Load and validate a specific file.

`Config::validate()`: Validate names, versions, packages, indexes, and tasks.

`Config::add_packages(path, section, packages)`: Add packages to `dual.toml`.

`Config::remove_packages(path, section, packages)`: Remove packages from `dual.toml`.

`Config::enable_language(path, section, version)`: Add a missing R or Python
section or update its runtime version after validating the prospective config.

`Config::disable_language(path, section, force)`: Safely remove an R or Python
section, enforcing dependency, task, and last-runtime guards.

`starter_config(project_name, python, r)`: Render and validate a new mixed- or single-language project configuration.

`validate_project_name(name)`: Validate a project name.

`validate_index_url(url)`: Validate an HTTP(S) package index URL.

`parse_python_requirement(requirement)`: Parse supported Python requirement syntax.

`valid_version_specifier(value)`: Validate version-specifier characters.

`valid_r_package_reference(package)`: Validate CRAN, Bioconductor, or GitHub R references.

### Metadata API

#### Name

`dual::metadata`

#### Usage

```rust
use dual::metadata::{ScriptKind, ScriptMetadata};

let kind = ScriptKind::from_path(std::path::Path::new("analysis.py"))?;
let block = dual::metadata::render(&ScriptMetadata::default(), kind);
```

#### Main Types

`ScriptKind`: `Python`, `R`, `Quarto`, or `RMarkdown`.

`ScriptMetadata`: Inline runtime versions, Python dependencies, Python indexes, and R package sources.

`ParsedMetadata`: Parsed metadata plus its source range.

`AddOptions`: Options for adding inline dependencies.

`ScriptLanguage`: `Python` or `R`.

#### Main Functions

`read(path)`: Read and parse inline metadata from a file.

`parse(contents, kind)`: Parse metadata from a string.

`initialize(path, python, r, force)`: Create or replace inline metadata.

`add(path, options)`: Add dependencies or indexes to inline metadata.

`render(metadata, kind)`: Render metadata back to script comments.

`absolute_path(path)`: Resolve a path against the current directory.

### Workflow API

#### Name

`dual::workflows`

#### Usage

```rust
use dual::workflows::{ExportFormat, export, sync_project};

sync_project(root, false, true, false)?;
let path = export(root, ExportFormat::Requirements)?;
```

#### Main Types

`ExportFormat`: `Requirements`, `Renv`, or `Dockerfile`.

#### Main Functions

`looks_like_script(target)`: Detect supported script/document paths.

`run_script(path, args, verbose, trust_project, no_install, dry_run)`: Run a script workflow.

`sync_script(path, verbose, trust_project, dry_run)`: Prepare a script environment.

`sync_project(root, verbose, trust_project, dry_run)`: Prepare a project environment.

`show_script_dependencies(path, json)`: Print script dependencies.

`show_project_dependencies(root, json)`: Print project dependencies.

`print_dependencies(config, source)`: Print dependencies for a `Config`.

`export(root, format)`: Write a dependency export artifact.

### Task API

#### Name

`dual::tasks`

#### Usage

```rust
let task = dual::tasks::lookup(&config, "analysis")?;
println!("{}", task.command());
```

#### Main Functions

`lookup(config, name)`: Find a task or return an error listing available tasks.

`run_task(root, backend, name, args, trust_project)`: Run a task and its dependencies.

`list_tasks(root, json)`: Print configured tasks.

### Import API

#### Name

`dual::imports`

#### Usage

```rust
let report = dual::imports::import_file(root, std::path::Path::new("requirements.txt"))?;
```

#### Main Types

`ImportReport`: Source path, imported Python packages, imported R packages, runtime versions, and skipped entries.

#### Main Functions

`import_file(project_root, source)`: Parse a supported dependency file and apply supported entries to `dual.toml`.

### Backend API

#### Name

`dual::backend`

#### Usage

```rust
use dual::backend::{Backend, EnvironmentBackend};

let backend = EnvironmentBackend::new(root, false);
backend.ensure_available()?;
```

#### Main Types

`Backend`: Trait implemented by environment backends.

`EnvironmentBackend`: Default backend implementation.

`BackendReport`: Environment diagnostic report.

`BridgeReport`: R/Python bridge diagnostic report.

#### Main Functions

`EnvironmentBackend::new(root, verbose)`: Create a project backend.

`EnvironmentBackend::for_script(root, script, verbose)`: Create a script-specific backend.

`generate_manifest(config, root)`: Generate the backend manifest from a `Config`.

#### Backend Trait Methods

`is_available()`: Return whether environment support is installed.

`ensure_available()`: Install or verify environment support.

`update_engine()`: Download and activate the pinned engine.

`uninstall_engine()`: Remove private engine support.

`migrate_lock()`: Migrate the public lockfile format.

`environment_exists()`: Check whether the generated environment is present.

`verify_manifest(config)`: Ensure generated state matches `dual.toml`.

`init_or_update(config, refresh)`: Create or update the environment.

`validate(config)`: Validate runtime and package availability.

`run(config, task, args)`: Run a configured backend task.

`shell(config)`: Open an interactive environment shell.

`clean()`: Remove generated backend files.

`doctor(config)`: Return diagnostic information.

### Security API

#### Name

`dual::security`

#### Usage

```rust
let trust = dual::security::ensure_project_trusted(root, true)?;
dual::security::verify_project_unchanged(root, &trust)?;
```

#### Main Types

`ProjectTrust`: Snapshot of the project state after trust is granted.

`ProjectSnapshot`: Snapshot that allows selected output paths to change.

#### Main Constants

`MAX_CONFIG_BYTES`: Maximum size for configuration-like text inputs.

`MAX_LOCK_BYTES`: Maximum size for lockfile inputs.

#### Main Functions

`default_dual_home()`: Return `DUAL_HOME` or the platform default data directory.

`read_text_file(path, limit, label)`: Read a UTF-8 file with size and symlink checks.

`read_file(path, limit, label)`: Read bytes with size and symlink checks.

`write_file_atomic(path, contents, label)`: Atomically write a file without following symlinks.

`reject_symlink(path, label)`: Reject an existing symlink.

`reject_symlink_if_present(path, label)`: Reject a symlink if the path exists.

`ensure_managed_path(root, path)`: Ensure generated state stays under the project root.

`contains_control_characters(value)`: Detect control and bidi control characters.

`ensure_project_trusted(root, authorize)`: Verify or grant trust.

`project_is_trusted(root)`: Check trust without granting it.

`refresh_project_trust(root)`: Refresh the stored trust record.

`verify_project_unchanged(root, trust)`: Ensure source files did not change during execution.

`snapshot_project_excluding(root, excluded)`: Snapshot a project while allowing specific outputs.

`verify_project_snapshot(root, snapshot)`: Verify a scoped snapshot.

`create_private_directory(path, label)`: Create a private data directory.

### Diagnostics and Platform API

#### Name

`dual::doctor`, `dual::platform`, and `dual::errors`

#### Usage

```rust
dual::doctor::run(root, &backend, false)?;
let script = dual::platform::referenced_script("Rscript scripts/analysis.R");
```

#### Main Types

`DualError`: User-facing error variants for missing configs, invalid configs, missing tasks, and backend startup/failure.

#### Main Functions

`doctor::run(root, backend, json)`: Run project diagnostics.

`doctor::run_system(json)`: Run diagnostics when no project is present.

`platform::managed_paths(root)`: Return paths generated by `dual`.

`platform::referenced_script(command)`: Extract a referenced script/document from a task command.

`ProjectEnvironment::load(root)`: Safely parse the project-root `.env` without
modifying the current process.

`ProjectEnvironment::apply_to_command(command)`: Add project values that are
not already defined by the invoking environment or command.

## Environment Variables

`DUAL_HOME`: Override the data directory used for trust records and private engine support.

`DUAL_TRUST_PROJECT`: Set to `1`, `true`, or `yes` to authorize trust noninteractively.

`DUAL_ALLOW_CREDENTIALS`: Set to `1`, `true`, or `yes` to allow common credential variables through to package tooling.

`DUAL_ENGINE_PATH`: Use a specific environment engine executable.

`DUAL_ENGINE_DOWNLOAD_URL`: Override the engine download URL.

`DUAL_ENGINE_SHA256`: Override the expected engine checksum.

`DUAL_USER_ZDOTDIR`: Used when preparing managed zsh startup behavior for `dual shell`.

## Files

`dual.toml`: Main project configuration.

`dual.lock`: Public shared lockfile.

`.dual/`: Generated project environment state.

`.dual/scripts/<hash>/`: Generated state for script-specific environments inside a project.

`.env`: Optional local variables for tasks, scripts, documents, and `dual shell`.

`.env.example`: Recommended secret-free project environment template.

`requirements.txt`: Python export target.

`renv-dependencies.R`: R export helper target.

`Dockerfile` and `.dockerignore`: Docker export targets.

## Safety Notes

Package installation, task commands, lockfiles, inline metadata, `.env`, and interactive shells can affect code running with the current user's permissions. `dual` therefore requires project trust before installation or execution, rejects symbolic links for sensitive paths, fingerprints project files, checks for changes during execution, reserves control-plane environment names, and removes common credential variables from package-tool subprocesses by default.

Generated document outputs for Quarto and R Markdown are allowed to change during rendering, but source files are still checked after execution.

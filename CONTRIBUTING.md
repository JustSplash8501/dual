# Contributing to dual

Thanks for helping make scientific projects easier to reproduce.

## Before you start

- Search existing GitHub issues and pull requests before opening a duplicate.
- For substantial behavior or configuration changes, open an issue first so
  the design can be discussed before implementation.
- Keep the MVP focused: dual coordinates project environments and tasks; it
  does not reimplement a package manager.

## Development setup

Requirements:

- Git
- Rust 1.80 or newer

Clone the repository and build the CLI:

```console
git clone https://github.com/OWNER/dual.git
cd dual
cargo build
```

Environment support is installed privately by dual when a real environment is
first created. R and Python do not need to be installed globally.

## Making changes

Create a branch from `main`:

```console
git switch main
git pull --ff-only
git switch -c feature/short-description
```

Please:

- Keep user-facing configuration in `dual.toml`.
- Keep generated local state under `.dual/`.
- Preserve `dual.lock` compatibility or provide an explicit migration.
- Avoid exposing internal environment-engine commands, names, or files.
- Treat Linux, macOS, and Windows as first-class platforms.
- Add or update tests for behavior changes.
- Update the README when commands, configuration, or installation behavior
  changes.

## Required checks

Run these before opening a pull request:

```console
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release
```

For changes affecting environments, locks, tasks, R/Python interoperability,
or installation, also run the real integration test:

```console
export DUAL_BIN="$PWD/target/release/dual"
export DUAL_HOME="$(mktemp -d)"
export DUAL_ENGINE_DISABLE_PATH_FALLBACK=1
bash scripts/ci/real-integration.sh
```

On Windows PowerShell:

```powershell
$env:DUAL_BIN = "$PWD\target\release\dual.exe"
$env:DUAL_HOME = Join-Path $env:TEMP "dual-integration-home"
$env:DUAL_ENGINE_DISABLE_PATH_FALLBACK = "1"
.\scripts\ci\real-integration.ps1
```

The integration test downloads environment components and may take several
minutes.

## Pull requests

Push your branch and open a pull request against `main`:

```console
git push -u origin feature/short-description
```

In the pull request:

- Explain the user-facing problem and the chosen solution.
- List the tests you ran.
- Call out configuration or lockfile compatibility changes.
- Include platform-specific notes when relevant.
- Keep unrelated refactors out of the same pull request.

GitHub Actions must pass formatting, linting, unit tests, release builds, and
real environment integration tests.

## Reporting bugs

Include:

- The dual version (`dual --version`)
- Operating system and architecture
- The command that failed
- A minimal `dual.toml`, with secrets removed
- Relevant output from `dual doctor`
- Whether the problem reproduces after `dual clean --yes` and `dual up`

Do not attach `.dual/` environments or credentials. Share `dual.lock` only if
it does not contain private package locations or credentials.

## Security issues

Please do not publicly disclose vulnerabilities involving automatic downloads,
checksum verification, command execution, or credential exposure. Use the
repository's private GitHub security advisory form when available.

## License

By contributing, you agree that your contributions are licensed under the MIT
License.

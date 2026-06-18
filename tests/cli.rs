use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn init_creates_expected_files() {
    let directory = tempdir().unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "--name", "experiment"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created dual.toml"));

    let config = fs::read_to_string(directory.path().join("dual.toml")).unwrap();
    assert!(config.contains("name = \"experiment\""));
    assert!(directory.path().join("scripts").is_dir());
    assert!(directory.path().join("data").is_dir());
    assert!(directory.path().join("results").is_dir());
}

#[test]
fn init_does_not_overwrite_existing_config() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("dual.toml");
    fs::write(&path, "sentinel").unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    assert_eq!(fs::read_to_string(path).unwrap(), "sentinel");
}

#[test]
fn add_r_updates_packages_without_duplicates() {
    let directory = initialized_project();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["add", "r", "tidyverse", "lme4", "tidyverse"])
        .assert()
        .success();

    let config = fs::read_to_string(directory.path().join("dual.toml")).unwrap();
    assert!(config.contains("packages = [\"tidyverse\", \"lme4\"]"));
}

#[test]
fn add_r_accepts_cran_bioconductor_and_github_references() {
    let directory = initialized_project();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "add",
            "r",
            "cran::targets@1.11.4",
            "bioc::DESeq2",
            "github::r-lib/pak@v0.9.0",
        ])
        .assert()
        .success();

    let config = fs::read_to_string(directory.path().join("dual.toml")).unwrap();
    assert!(config.contains("cran::targets@1.11.4"));
    assert!(config.contains("bioc::DESeq2"));
    assert!(config.contains("github::r-lib/pak@v0.9.0"));
}

#[test]
fn add_py_updates_packages() {
    let directory = initialized_project();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["add", "py", "pandas", "scikit-learn"])
        .assert()
        .success();

    let config = fs::read_to_string(directory.path().join("dual.toml")).unwrap();
    assert!(config.contains("packages = [\"pandas\", \"scikit-learn\"]"));
}

#[test]
fn missing_task_has_useful_error() {
    let directory = initialized_project();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["run", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No tasks are configured"));
}

#[test]
fn clean_removes_environment_but_preserves_shared_lockfile() {
    let directory = initialized_project();
    write_test_lock(directory.path(), "lock");
    fs::create_dir(directory.path().join(".dual")).unwrap();
    fs::write(directory.path().join(".dual").join("ready"), "ready").unwrap();
    fs::write(
        directory.path().join("scripts").join("keep.R"),
        "print('keep')",
    )
    .unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["clean", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Removed generated environment files",
        ));

    assert!(directory.path().join("dual.lock").exists());
    assert!(!directory.path().join(".dual").exists());
    assert!(directory.path().join("dual.toml").exists());
    assert!(directory.path().join("scripts").join("keep.R").exists());
}

#[test]
fn clean_with_only_a_shared_lockfile_is_safe() {
    let directory = initialized_project();
    write_test_lock(directory.path(), "lock");

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["clean", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to clean"));

    assert!(directory.path().join("dual.lock").exists());
}

#[test]
fn clean_does_not_remove_user_files() {
    let directory = initialized_project();
    fs::create_dir(directory.path().join(".dual")).unwrap();
    fs::write(directory.path().join("notes.txt"), "keep").unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["clean", "--yes"])
        .assert()
        .success();

    assert!(directory.path().join("notes.txt").exists());
}

#[cfg(unix)]
#[test]
fn up_enforces_an_existing_shared_lockfile() {
    let fixture = backend_fixture();
    write_test_lock(fixture.project.path(), "shared lock");

    dual_command(&fixture)
        .arg("up")
        .assert()
        .success()
        .stdout(predicate::str::contains("Project is ready"));

    let log = fs::read_to_string(&fixture.log).unwrap();
    assert!(log.lines().any(|line| line.starts_with("install ")));
    assert!(log.contains("--manifest-path"));
    assert!(log.contains("--locked"));
    assert!(!fixture
        .project
        .path()
        .join(".dual/workspace/pixi.lock")
        .exists());
}

#[cfg(unix)]
#[test]
fn up_refresh_intentionally_updates_the_shared_lockfile() {
    let fixture = backend_fixture();
    fs::write(fixture.project.path().join("dual.lock"), "old lock").unwrap();

    dual_command(&fixture)
        .args(["up", "--refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Project is ready"));

    let log = fs::read_to_string(&fixture.log).unwrap();
    let install = log
        .lines()
        .find(|line| line.starts_with("install "))
        .unwrap();
    assert!(!install.contains("--locked"));
    assert!(fixture.project.path().join("dual.lock").is_file());
}

#[cfg(unix)]
#[test]
fn stale_shared_lock_explains_how_to_refresh() {
    let fixture = backend_fixture();
    write_test_lock(fixture.project.path(), "stale lock");

    dual_command(&fixture)
        .env("DUAL_ENGINE_FAIL_LOCKED", "1")
        .arg("up")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("shared lockfile does not match dual.toml")
                .and(predicate::str::contains("dual up --refresh")),
        );
}

#[cfg(unix)]
#[test]
fn up_embeds_source_backed_r_packages_in_dual_lock() {
    let fixture = backend_fixture();
    dual_command(&fixture)
        .args(["add", "r", "cran::targets@1.11.4"])
        .assert()
        .success();

    dual_command(&fixture).arg("up").assert().success();

    let manifest = fs::read_to_string(
        fixture
            .project
            .path()
            .join(".dual/workspace/pyproject.toml"),
    )
    .unwrap();
    assert!(manifest.contains("r-pak"));
    assert!(!manifest.contains("r-targets"));

    let lock = fs::read_to_string(fixture.project.path().join("dual.lock")).unwrap();
    let lock: serde_json::Value = serde_json::from_str(&lock).unwrap();
    assert_eq!(lock["version"], 1);
    assert_eq!(lock["pixi"], "version: 6\n");
    assert_eq!(lock["r"]["sources"][0], "cran::targets@1.11.4");
    assert_eq!(lock["r"]["pak"]["lockfile_version"], "1.0.0");
    assert!(!fixture.project.path().join("dual-r.lock").exists());
    assert!(!fixture
        .project
        .path()
        .join(".dual/workspace/pak.lock")
        .exists());
}

#[cfg(unix)]
#[test]
fn stale_embedded_r_source_resolution_explains_how_to_refresh() {
    let fixture = backend_fixture();
    dual_command(&fixture)
        .args(["add", "r", "cran::targets@1.11.4"])
        .assert()
        .success();
    dual_command(&fixture).arg("up").assert().success();

    let config_path = fixture.project.path().join("dual.toml");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path)
            .unwrap()
            .replace("cran::targets@1.11.4", "bioc::DESeq2"),
    )
    .unwrap();

    dual_command(&fixture).arg("up").assert().failure().stderr(
        predicate::str::contains("dual.lock does not match dual.toml")
            .and(predicate::str::contains("dual up --refresh")),
    );
}

#[cfg(unix)]
#[test]
fn run_prints_task_output() {
    let fixture = backend_fixture();
    fs::write(
        fixture.project.path().join("dual.toml"),
        fs::read_to_string(fixture.project.path().join("dual.toml"))
            .unwrap()
            .replace("[tasks]\n", "[tasks]\nanalysis = \"python script.py\"\n"),
    )
    .unwrap();
    fs::create_dir_all(fixture.project.path().join(".dual/workspace")).unwrap();
    fs::write(
        fixture
            .project
            .path()
            .join(".dual/workspace/pyproject.toml"),
        "# This file is generated by dual.\n# Edit dual.toml instead.\n",
    )
    .unwrap();
    fs::write(fixture.project.path().join(".dual/ready"), "ready").unwrap();
    write_test_lock(fixture.project.path(), "lock");

    dual_command(&fixture)
        .env("DUAL_FAKE_TASK_OUTPUT", "analysis complete")
        .args(["run", "analysis"])
        .assert()
        .success()
        .stdout(predicate::str::contains("analysis complete"));
}

#[cfg(unix)]
#[test]
fn up_automatically_installs_private_environment_support() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = backend_fixture();
    let installer = fixture.bin.path().join("install-engine.sh");
    fs::write(
        &installer,
        r#"#!/bin/sh
mkdir -p "$PIXI_BIN_DIR"
cp "$DUAL_FAKE_ENGINE_SOURCE" "$PIXI_BIN_DIR/pixi"
chmod +x "$PIXI_BIN_DIR/pixi"
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&installer).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&installer, permissions).unwrap();
    let home = fixture.project.path().join("dual-home");
    let installer_url = format!("file://{}", installer.display());

    let mut command = Command::cargo_bin("dual").unwrap();
    command
        .current_dir(fixture.project.path())
        .env("DUAL_HOME", &home)
        .env("DUAL_ENGINE_DISABLE_PATH_FALLBACK", "1")
        .env("DUAL_ENGINE_INSTALLER_URL", installer_url)
        .env("DUAL_FAKE_ENGINE_SOURCE", &fixture.engine)
        .env("DUAL_ENGINE_LOG", &fixture.log)
        .arg("up")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Installing environment support")
                .and(predicate::str::contains("Environment support installed"))
                .and(predicate::str::contains("Project is ready")),
        );

    assert!(home.join("engine/bin/dual-engine").is_file());
    assert!(!home.join("engine/bin/pixi").exists());
    assert!(fixture.project.path().join("dual.lock").is_file());
}

#[cfg(unix)]
#[test]
fn automatic_install_failure_has_a_user_facing_error() {
    let directory = initialized_project();
    let home = directory.path().join("dual-home");
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .env("DUAL_HOME", &home)
        .env("DUAL_ENGINE_DISABLE_PATH_FALLBACK", "1")
        .env(
            "DUAL_ENGINE_INSTALLER_URL",
            "file:///definitely/missing/dual-installer.sh",
        )
        .arg("up")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Installing environment support"))
        .stderr(
            predicate::str::contains("check your network connection")
                .and(predicate::str::contains("pixi").not()),
        );
}

fn initialized_project() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .arg("init")
        .assert()
        .success();
    directory
}

fn write_test_lock(project: &std::path::Path, pixi: &str) {
    let lock = serde_json::json!({
        "version": 1,
        "pixi": pixi,
    });
    fs::write(
        project.join("dual.lock"),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
}

#[cfg(unix)]
struct BackendFixture {
    project: tempfile::TempDir,
    bin: tempfile::TempDir,
    log: std::path::PathBuf,
    engine: std::path::PathBuf,
}

#[cfg(unix)]
fn backend_fixture() -> BackendFixture {
    use std::os::unix::fs::PermissionsExt;

    let project = initialized_project();
    let bin = tempdir().unwrap();
    let log = project.path().join("engine-calls.log");
    let engine = bin.path().join("dual-engine");
    fs::write(
        &engine,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DUAL_ENGINE_LOG"
command_name=""
manifest=""
previous=""
for argument in "$@"; do
  if [ "$previous" = "--manifest-path" ]; then
    manifest="$argument"
  fi
  case "$argument" in
    install|run|shell) command_name="$argument" ;;
  esac
  previous="$argument"
done
if [ "$command_name" = "install" ]; then
  if [ "${DUAL_ENGINE_FAIL_LOCKED:-0}" = "1" ] && printf '%s\n' "$*" | grep -q -- '--locked'; then
    exit 1
  fi
  mkdir -p "$(dirname "$manifest")"
  printf 'version: 6\n' > "$(dirname "$manifest")/pixi.lock"
  exit 0
fi
if [ "$command_name" = "run" ]; then
  if printf '%s\n' "$*" | grep -q -- 'lockfile_create'; then
    mkdir -p .dual/workspace
    printf '{"lockfile_version":"1.0.0","packages":[]}\n' > .dual/workspace/pak.lock
  fi
  if [ -n "${DUAL_FAKE_TASK_OUTPUT:-}" ]; then
    printf '%s\n' "$DUAL_FAKE_TASK_OUTPUT"
  fi
  exit 0
fi
if [ "$command_name" = "shell" ]; then
  exit 0
fi
exit 1
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&engine).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&engine, permissions).unwrap();

    BackendFixture {
        project,
        bin,
        log,
        engine,
    }
}

#[cfg(unix)]
fn dual_command(fixture: &BackendFixture) -> Command {
    let mut command = Command::cargo_bin("dual").unwrap();
    command
        .current_dir(fixture.project.path())
        .env("DUAL_ENGINE_PATH", &fixture.engine)
        .env("DUAL_ENGINE_LOG", &fixture.log);
    command
}

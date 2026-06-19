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
        .args(["init", "experiment"])
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
fn init_infers_project_name_from_directory() {
    let parent = tempdir().unwrap();
    let directory = parent.path().join("inferred-project");
    fs::create_dir(&directory).unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(&directory)
        .arg("init")
        .assert()
        .success();

    let config = fs::read_to_string(directory.join("dual.toml")).unwrap();
    assert!(config.contains("name = \"inferred-project\""));
}

#[test]
fn init_rejects_unsafe_project_names() {
    let invalid = [
        "has spaces",
        "café",
        "-leading",
        "trailing-",
        "slash/name",
        "dot.name",
    ];
    for name in invalid {
        let directory = tempdir().unwrap();
        Command::cargo_bin("dual")
            .unwrap()
            .current_dir(directory.path())
            .args(["init", "--", name])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "project name must start and end with a letter or number",
            ));
        assert!(!directory.path().join("dual.toml").exists());
    }
}

#[test]
fn init_does_not_overwrite_existing_config() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("dual.toml");
    fs::write(&path, "sentinel").unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "existing-project"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    assert_eq!(fs::read_to_string(path).unwrap(), "sentinel");
}

#[test]
fn force_init_invalidates_stale_environment_and_lock() {
    let directory = initialized_project();
    fs::create_dir(directory.path().join(".dual")).unwrap();
    write_test_lock(directory.path(), "old resolution");

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "replacement", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Invalidated the previous environment and lockfile",
        ));

    assert!(!directory.path().join(".dual").exists());
    assert!(!directory.path().join("dual.lock").exists());
    assert!(fs::read_to_string(directory.path().join("dual.toml"))
        .unwrap()
        .contains("name = \"replacement\""));
}

#[test]
fn commands_find_project_from_subdirectories() {
    let directory = initialized_project();
    let nested = directory.path().join("scripts/nested");
    fs::create_dir_all(&nested).unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(&nested)
        .args(["add", "py", "pandas>=2"])
        .assert()
        .success();

    assert!(fs::read_to_string(directory.path().join("dual.toml"))
        .unwrap()
        .contains("pandas>=2"));
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
        .args(["add", "py", "pandas>=2,<3", "requests[socks]==2.32.3"])
        .assert()
        .success();

    let config = fs::read_to_string(directory.path().join("dual.toml")).unwrap();
    assert!(config.contains("packages = [\"pandas>=2,<3\", \"requests[socks]==2.32.3\"]"));
}

#[test]
fn remove_packages_updates_config() {
    let directory = initialized_project();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["add", "py", "pandas", "numpy"])
        .assert()
        .success();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["remove", "py", "pandas"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 package"));

    let config = fs::read_to_string(directory.path().join("dual.toml")).unwrap();
    assert!(!config.contains("\"pandas\""));
    assert!(config.contains("\"numpy\""));
}

#[test]
fn task_list_prints_configured_tasks() {
    let directory = initialized_project();
    let path = directory.path().join("dual.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("[tasks]\n", "[tasks]\nanalysis = \"Rscript analysis.R\"\n"),
    )
    .unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["task", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("analysis\tRscript analysis.R"));
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
fn add_rejects_a_symlinked_config() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    let outside = directory.path().join("outside.toml");
    fs::write(&outside, dual::config::DEFAULT_CONFIG).unwrap();
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    symlink(&outside, project.join("dual.toml")).unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(&project)
        .args(["add", "py", "requests"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must not be a symbolic link"));

    assert_eq!(
        fs::read_to_string(outside).unwrap(),
        dual::config::DEFAULT_CONFIG
    );
}

#[cfg(unix)]
#[test]
fn up_rejects_a_symlinked_state_directory() {
    use std::os::unix::fs::symlink;

    let fixture = backend_fixture();
    let outside = fixture.project.path().join("outside-state");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.project.path().join(".dual")).unwrap();

    dual_command(&fixture)
        .arg("up")
        .assert()
        .failure()
        .stderr(predicate::str::contains("must not be a symbolic link"));

    assert!(fs::read_dir(outside).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn execution_requires_trust_and_config_changes_invalidate_it() {
    let fixture = backend_fixture();
    let home = fixture.project.path().join("trust-home");

    untrusted_dual_command(&fixture, &home)
        .arg("up")
        .assert()
        .failure()
        .stderr(predicate::str::contains("project is not trusted"));

    untrusted_dual_command(&fixture, &home)
        .args(["--trust-project", "up"])
        .assert()
        .success();

    let config_path = fixture.project.path().join("dual.toml");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path)
            .unwrap()
            .replace("name = \"my-project\"", "name = \"changed\""),
    )
    .unwrap();

    untrusted_dual_command(&fixture, &home)
        .arg("up")
        .assert()
        .failure()
        .stderr(predicate::str::contains("project files changed"));
}

#[cfg(unix)]
#[test]
fn script_changes_invalidate_project_trust() {
    let fixture = backend_fixture();
    let home = fixture.engine.parent().unwrap().join("script-trust-home");
    let config_path = fixture.project.path().join("dual.toml");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path).unwrap().replace(
            "[tasks]\n",
            "[tasks]\nanalysis = \"python scripts/analysis.py\"\n",
        ),
    )
    .unwrap();
    fs::write(
        fixture.project.path().join("scripts/analysis.py"),
        "print('safe')\n",
    )
    .unwrap();

    untrusted_dual_command(&fixture, &home)
        .args(["--trust-project", "up"])
        .assert()
        .success();

    fs::write(
        fixture.project.path().join("scripts/analysis.py"),
        "print('changed')\n",
    )
    .unwrap();

    untrusted_dual_command(&fixture, &home)
        .args(["run", "analysis"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("project is not trusted"));
}

#[cfg(unix)]
#[test]
fn tampered_generated_manifest_is_rejected() {
    let fixture = backend_fixture();
    dual_command(&fixture).arg("up").assert().success();

    let manifest = fixture
        .project
        .path()
        .join(".dual/workspace/pyproject.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("cmd = ", "cmd = \"touch exploited\" # "),
    )
    .unwrap();

    dual_command(&fixture)
        .args(["run", "missing"])
        .assert()
        .failure();

    let config_path = fixture.project.path().join("dual.toml");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path)
            .unwrap()
            .replace("[tasks]\n", "[tasks]\nanalysis = \"echo safe\"\n"),
    )
    .unwrap();
    dual_command(&fixture)
        .args(["--trust-project", "run", "analysis"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "generated environment manifest does not match",
        ));
}

#[cfg(unix)]
#[test]
fn execution_time_project_mutation_is_not_trusted() {
    let fixture = backend_fixture();
    let home = fixture.engine.parent().unwrap().join("mutation-trust-home");

    untrusted_dual_command(&fixture, &home)
        .env("DUAL_ENGINE_MUTATE_CONFIG", "1")
        .args(["--trust-project", "up"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Project files changed while code was executing",
        ));

    untrusted_dual_command(&fixture, &home)
        .arg("up")
        .assert()
        .failure()
        .stderr(predicate::str::contains("project is not trusted"));
}

#[cfg(unix)]
#[test]
fn shell_requires_a_complete_environment() {
    let fixture = backend_fixture();
    dual_command(&fixture)
        .arg("shell")
        .assert()
        .failure()
        .stderr(predicate::str::contains("environment has not been created"));
}

#[cfg(unix)]
#[test]
fn shell_uses_the_project_name_prompt() {
    let fixture = backend_fixture();
    dual_command(&fixture).arg("up").assert().success();
    let profile = fs::read_to_string(fixture.project.path().join(".dual/Rprofile")).unwrap();
    assert!(profile.contains(&format!(
        "- Project '%s' loaded. [dual {}]",
        env!("CARGO_PKG_VERSION")
    )));

    dual_command(&fixture).arg("shell").assert().success();

    let log = fs::read_to_string(&fixture.log).unwrap();
    assert!(
        log.lines().any(|line| {
            line.contains("shell")
                && line.contains("--locked")
                && line.contains("--change-ps1 true")
        }),
        "shell did not request the project prompt: {log}"
    );
}

#[cfg(unix)]
#[test]
fn nested_shells_exit_cleanly_and_remove_staged_state() {
    let fixture = backend_fixture();
    dual_command(&fixture).arg("up").assert().success();

    dual_command(&fixture)
        .env("DUAL_ENGINE_NESTED_SHELL", "1")
        .env("DUAL_BIN", assert_cmd::cargo::cargo_bin!("dual"))
        .arg("shell")
        .assert()
        .success();

    let log = fs::read_to_string(&fixture.log).unwrap();
    assert_eq!(
        log.lines()
            .filter(|line| line.split_whitespace().any(|part| part == "shell"))
            .count(),
        2
    );
    assert!(!fixture
        .project
        .path()
        .join(".dual/workspace/pixi.lock")
        .exists());
}

#[cfg(unix)]
#[test]
fn r_checks_disable_startup_profiles() {
    let fixture = backend_fixture();
    dual_command(&fixture).arg("up").assert().success();

    let log = fs::read_to_string(&fixture.log).unwrap();
    for line in log.lines().filter(|line| line.contains("Rscript")) {
        assert!(
            line.contains("Rscript --vanilla"),
            "R invocation did not disable startup files: {line}"
        );
        assert!(
            !line.contains("Rscript --vanilla --version"),
            "Rscript treats --version after --vanilla as a missing script: {line}"
        );
    }
}

#[cfg(unix)]
#[test]
fn environment_preparation_does_not_inherit_common_credentials() {
    let fixture = backend_fixture();
    let credential_log = fixture.engine.parent().unwrap().join("credential-leak");

    dual_command(&fixture)
        .env("GITHUB_TOKEN", "top-secret")
        .env("DUAL_ENGINE_CREDENTIAL_LOG", &credential_log)
        .arg("up")
        .assert()
        .success();

    assert!(!credential_log.exists());
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
    assert_eq!(lock["environment"], "version: 6\n");
    assert!(lock.get("pixi").is_none());
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
    write_ready_environment(fixture.project.path());
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
fn failed_task_preserves_useful_stderr_without_engine_branding() {
    let fixture = backend_fixture();
    fs::write(
        fixture.project.path().join("dual.toml"),
        fs::read_to_string(fixture.project.path().join("dual.toml"))
            .unwrap()
            .replace("[tasks]\n", "[tasks]\nanalysis = \"python script.py\"\n"),
    )
    .unwrap();
    fs::create_dir_all(fixture.project.path().join(".dual/workspace")).unwrap();
    write_ready_environment(fixture.project.path());
    write_test_lock(fixture.project.path(), "lock");

    dual_command(&fixture)
        .env("DUAL_FAKE_TASK_ERROR", "analysis exploded")
        .args(["run", "analysis"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("analysis exploded")
                .and(predicate::str::contains("pixi").not()),
        );
}

#[cfg(unix)]
#[test]
fn up_automatically_installs_private_environment_support() {
    let fixture = backend_fixture();
    let home = fixture.engine.parent().unwrap().join("dual-home");
    let download_url = format!("file://{}", fixture.engine.display());
    let checksum = sha256(&fs::read(&fixture.engine).unwrap());

    let mut command = Command::cargo_bin("dual").unwrap();
    command
        .current_dir(fixture.project.path())
        .env("DUAL_HOME", &home)
        .env("DUAL_ENGINE_DISABLE_PATH_FALLBACK", "1")
        .env("DUAL_ENGINE_DOWNLOAD_URL", download_url)
        .env("DUAL_ENGINE_SHA256", checksum)
        .env("DUAL_ENGINE_LOG", &fixture.log)
        .env("DUAL_TRUST_PROJECT", "1")
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
        .env("DUAL_TRUST_PROJECT", "1")
        .env(
            "DUAL_ENGINE_DOWNLOAD_URL",
            "file:///definitely/missing/dual-engine",
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

#[cfg(unix)]
#[test]
fn automatic_install_rejects_a_bad_checksum() {
    let fixture = backend_fixture();
    let home = fixture.project.path().join("dual-home");
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(fixture.project.path())
        .env("DUAL_HOME", &home)
        .env("DUAL_ENGINE_DISABLE_PATH_FALLBACK", "1")
        .env("DUAL_TRUST_PROJECT", "1")
        .env(
            "DUAL_ENGINE_DOWNLOAD_URL",
            format!("file://{}", fixture.engine.display()),
        )
        .env("DUAL_ENGINE_SHA256", "0".repeat(64))
        .arg("up")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed its integrity check"));

    assert!(!home.join("engine/bin/dual-engine").exists());
}

#[cfg(unix)]
#[test]
fn managed_engine_is_reverified_before_execution() {
    let fixture = backend_fixture();
    let home = fixture
        .engine
        .parent()
        .unwrap()
        .join("verified-engine-home");
    let checksum = sha256(&fs::read(&fixture.engine).unwrap());

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(fixture.project.path())
        .env("DUAL_HOME", &home)
        .env("DUAL_ENGINE_DISABLE_PATH_FALLBACK", "1")
        .env(
            "DUAL_ENGINE_DOWNLOAD_URL",
            format!("file://{}", fixture.engine.display()),
        )
        .env("DUAL_ENGINE_SHA256", &checksum)
        .env("DUAL_ENGINE_LOG", &fixture.log)
        .env("DUAL_TRUST_PROJECT", "1")
        .arg("up")
        .assert()
        .success();

    let managed = home.join("engine/bin/dual-engine");
    fs::write(&managed, b"tampered").unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(fixture.project.path())
        .env("DUAL_HOME", &home)
        .env("DUAL_ENGINE_DISABLE_PATH_FALLBACK", "1")
        .env("DUAL_ENGINE_SHA256", checksum)
        .env("DUAL_TRUST_PROJECT", "1")
        .arg("up")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed its integrity check"));
}

#[cfg(unix)]
#[test]
fn engine_update_and_uninstall_manage_private_engine() {
    let fixture = backend_fixture();
    let home = fixture.project.path().join("dual-home");
    let download_url = format!("file://{}", fixture.engine.display());
    let checksum = sha256(&fs::read(&fixture.engine).unwrap());

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(fixture.project.path())
        .env("DUAL_HOME", &home)
        .env("DUAL_ENGINE_DOWNLOAD_URL", download_url)
        .env("DUAL_ENGINE_SHA256", checksum)
        .args(["engine", "update"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Environment support updated"));
    assert!(home.join("engine/bin/dual-engine").is_file());

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(fixture.project.path())
        .env("DUAL_HOME", &home)
        .args(["engine", "uninstall"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Removed dual's private environment support",
        ));
    assert!(!home.join("engine").exists());
}

#[test]
fn lock_migrate_rewrites_legacy_field() {
    let directory = initialized_project();
    fs::write(
        directory.path().join("dual.lock"),
        r#"{"version":1,"pixi":"legacy"}"#,
    )
    .unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["lock", "migrate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Migrated dual.lock"));

    let lock = fs::read_to_string(directory.path().join("dual.lock")).unwrap();
    assert!(lock.contains("\"environment\""));
    assert!(!lock.contains("\"pixi\""));
}

fn sha256(contents: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(contents))
}

fn initialized_project() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "test-project"])
        .assert()
        .success();
    directory
}

fn write_test_lock(project: &std::path::Path, environment: &str) {
    let lock = serde_json::json!({
        "version": 1,
        "environment": environment,
    });
    fs::write(
        project.join("dual.lock"),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
}

fn write_ready_environment(project: &std::path::Path) {
    let config = dual::config::Config::load(project).unwrap();
    let manifest = dual::backend::generate_manifest(&config).unwrap();
    fs::create_dir_all(project.join(".dual/workspace")).unwrap();
    fs::write(project.join(".dual/workspace/pyproject.toml"), manifest).unwrap();
    fs::write(project.join(".dual/ready"), "ready").unwrap();
}

#[cfg(unix)]
struct BackendFixture {
    project: tempfile::TempDir,
    _bin: tempfile::TempDir,
    log: std::path::PathBuf,
    engine: std::path::PathBuf,
}

#[cfg(unix)]
fn backend_fixture() -> BackendFixture {
    use std::os::unix::fs::PermissionsExt;

    let project = initialized_project();
    let bin = tempdir().unwrap();
    let log = bin.path().join("engine-calls.log");
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
  if [ -n "${GITHUB_TOKEN:-}" ] && [ -n "${DUAL_ENGINE_CREDENTIAL_LOG:-}" ]; then
    printf '%s\n' "$GITHUB_TOKEN" > "$DUAL_ENGINE_CREDENTIAL_LOG"
  fi
  if [ "${DUAL_ENGINE_FAIL_LOCKED:-0}" = "1" ] && printf '%s\n' "$*" | grep -q -- '--locked'; then
    exit 1
  fi
  mkdir -p "$(dirname "$manifest")"
  printf 'version: 6\n' > "$(dirname "$manifest")/pixi.lock"
  if [ "${DUAL_ENGINE_MUTATE_CONFIG:-0}" = "1" ] && ! grep -q '^evil =' dual.toml; then
    printf '%s\n' 'evil = "echo injected"' >> dual.toml
  fi
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
  if [ -n "${DUAL_FAKE_TASK_ERROR:-}" ]; then
    printf '%s\n' "$DUAL_FAKE_TASK_ERROR" >&2
    exit 1
  fi
  exit 0
fi
if [ "$command_name" = "shell" ]; then
  if [ "${DUAL_ENGINE_NESTED_SHELL:-0}" = "1" ] && [ "${DUAL_ENGINE_NESTED_GUARD:-0}" != "1" ]; then
    DUAL_ENGINE_NESTED_GUARD=1 "$DUAL_BIN" shell
  fi
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
        _bin: bin,
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
        .env("DUAL_ENGINE_LOG", &fixture.log)
        .env("DUAL_HOME", fixture.project.path().join("dual-home"))
        .env("DUAL_TRUST_PROJECT", "1");
    command
}

#[cfg(unix)]
fn untrusted_dual_command(fixture: &BackendFixture, home: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("dual").unwrap();
    command
        .current_dir(fixture.project.path())
        .env("DUAL_ENGINE_PATH", &fixture.engine)
        .env("DUAL_ENGINE_LOG", &fixture.log)
        .env("DUAL_HOME", home)
        .env_remove("DUAL_TRUST_PROJECT");
    command
}

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
fn init_script_creates_python_r_and_document_starters() {
    let directory = tempdir().unwrap();
    for (file, args, expected) in [
        (
            "analysis.py",
            vec!["--python", "3.12"],
            "requires-python = \"3.12\"",
        ),
        ("analysis.R", vec!["--r", "4.4"], "r = \"4.4\""),
        (
            "report.qmd",
            vec!["--python", "3.12", "--r", "4.4"],
            "<!-- /// script",
        ),
        (
            "report.Rmd",
            vec!["--python", "3.12", "--r", "4.4"],
            "<!-- /// script",
        ),
    ] {
        let mut command = Command::cargo_bin("dual").unwrap();
        command
            .current_dir(directory.path())
            .args(["init", "--script", file])
            .args(args)
            .assert()
            .success();
        let contents = fs::read_to_string(directory.path().join(file)).unwrap();
        assert!(contents.contains(expected));
        assert!(contents.contains("Hello from dual"));
    }
}

#[test]
fn init_script_preserves_shebang_and_requires_force_for_existing_metadata() {
    let directory = tempdir().unwrap();
    let script = directory.path().join("analysis.py");
    fs::write(&script, "#!/usr/bin/env -S dual run\nprint('existing')\n").unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "--script", "analysis.py"])
        .assert()
        .success();
    let contents = fs::read_to_string(&script).unwrap();
    assert!(contents.starts_with("#!/usr/bin/env -S dual run\n# /// script"));
    assert!(contents.contains("print('existing')"));

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "--script", "analysis.py"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already contains inline metadata"));
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "init",
            "--script",
            "analysis.py",
            "--python",
            "3.13",
            "--force",
        ])
        .assert()
        .success();
    assert!(fs::read_to_string(script)
        .unwrap()
        .contains("requires-python = \"3.13\""));
}

#[test]
fn add_script_updates_dependencies_sources_and_avoids_duplicates() {
    let directory = tempdir().unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "--script", "analysis.py"])
        .assert()
        .success();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "add",
            "--script",
            "analysis.py",
            "--index",
            "https://example.com/simple",
            "rich",
            "rich",
        ])
        .assert()
        .success();
    let contents = fs::read_to_string(directory.path().join("analysis.py")).unwrap();
    assert_eq!(contents.matches("\"rich\"").count(), 1);
    assert!(contents.contains("[[tool.dual.index]]"));
    assert!(contents.contains("https://example.com/simple"));

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["init", "--script", "report.qmd"])
        .assert()
        .success();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["add", "--script", "report.qmd", "knitr"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("use `--python` or `--r`"));
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["add", "--script", "report.qmd", "--r", "--bioc", "DESeq2"])
        .assert()
        .success();
    assert!(fs::read_to_string(directory.path().join("report.qmd"))
        .unwrap()
        .contains("bioc = [\n  \"DESeq2\","));
}

#[test]
fn deps_and_dry_run_merge_project_and_inline_metadata() {
    let directory = initialized_project();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["add", "py", "pandas"])
        .assert()
        .success();
    fs::write(
        directory.path().join("analysis.py"),
        "# /// script\n# requires-python = \">=3.13\"\n# dependencies = [\"rich\"]\n# ///\nprint('ok')\n",
    )
    .unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["deps", "--script", "analysis.py"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("project dual.toml and inline script metadata")
                .and(predicate::str::contains("Python version: >=3.13"))
                .and(predicate::str::contains("pandas, rich")),
        );
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["run", "analysis.py", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Would run: python \"analysis.py\"",
        ));
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["sync", "--script", "analysis.py", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would prepare dependencies"));
}

#[test]
fn deps_task_list_and_doctor_support_json() {
    let directory = initialized_project();
    let config_path = directory.path().join("dual.toml");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path).unwrap().replace(
            "[tasks]\n",
            "[tasks]\nprepare = \"python scripts/prepare.py\"\nanalysis = { cmd = \"python scripts/analysis.py\", deps = [\"prepare\"] }\n",
        ),
    )
    .unwrap();

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["--json", "deps"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"python\""));

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["--json", "task", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"deps\"").and(predicate::str::contains("prepare")));

    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["--json", "doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"config_present\": true"));
}

#[test]
fn export_commands_write_conservative_files() {
    let directory = initialized_project();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["add", "py", "pandas", "rich"])
        .assert()
        .success();
    for (flag, file, expected) in [
        ("--requirements", "requirements.txt", "pandas"),
        ("--renv", "renv-dependencies.R", "renv::init"),
        ("--dockerfile", "Dockerfile", "Generated by dual"),
    ] {
        Command::cargo_bin("dual")
            .unwrap()
            .current_dir(directory.path())
            .args(["export", flag])
            .assert()
            .success();
        assert!(fs::read_to_string(directory.path().join(file))
            .unwrap()
            .contains(expected));
    }
    let dockerfile = fs::read_to_string(directory.path().join("Dockerfile")).unwrap();
    assert!(dockerfile.contains("cat > /tmp/requirements.txt"));
    assert!(fs::read_to_string(directory.path().join(".dockerignore"))
        .unwrap()
        .contains(".dual/"));
}

#[test]
fn import_reads_supported_dependency_files() {
    let directory = initialized_project();
    fs::write(
        directory.path().join("requirements.txt"),
        "pandas==2.2.0\n# comment\n--extra-index-url https://example.com/simple\nrich\n",
    )
    .unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["--json", "import", "requirements.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pandas==2.2.0"));

    fs::write(
        directory.path().join("environment.yml"),
        "name: demo\ndependencies:\n  - python=3.12\n  - r-base=4.4\n  - r-dplyr=1.1.4\n  - pip:\n    - scikit-learn==1.5.0\n",
    )
    .unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["import", "environment.yml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Python version: 3.12"));

    fs::write(
        directory.path().join("uv.lock"),
        r#"requires-python = ">=3.12"

[[package]]
name = "numpy"
version = "2.0.0"
"#,
    )
    .unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["import", "uv.lock"])
        .assert()
        .success();

    fs::write(
        directory.path().join("renv.lock"),
        r#"{
  "R": { "Version": "4.4.0" },
  "Packages": {
    "targets": { "Package": "targets", "Version": "1.11.4", "Source": "CRAN" },
    "DESeq2": { "Package": "DESeq2", "Version": "1.42.0", "Source": "Bioconductor" }
  }
}"#,
    )
    .unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["import", "renv.lock"])
        .assert()
        .success();

    fs::write(
        directory.path().join("env.lock"),
        "packages:\n  - name: python\n    version: '3.12'\n  - name: r-ggplot2\n    version: 3.5.1\n",
    )
    .unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .args(["import", "env.lock"])
        .assert()
        .success();

    let config = fs::read_to_string(directory.path().join("dual.toml")).unwrap();
    assert!(config.contains("pandas==2.2.0"));
    assert!(config.contains("scikit-learn==1.5.0"));
    assert!(config.contains("numpy==2.0.0"));
    assert!(config.contains("dplyr@1.1.4"));
    assert!(config.contains("targets@1.11.4"));
    assert!(config.contains("DESeq2@1.42.0"));
    assert!(config.contains("ggplot2@3.5.1"));
}

#[test]
fn doctor_reports_system_status_without_a_project() {
    let directory = tempdir().unwrap();
    Command::cargo_bin("dual")
        .unwrap()
        .current_dir(directory.path())
        .arg("doctor")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("System")
                .and(predicate::str::contains("operating system"))
                .and(predicate::str::contains("no dual.toml found")),
        );
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
    assert!(config.contains("cran = [\"targets@1.11.4\"]"));
    assert!(config.contains("bioc = [\"DESeq2\"]"));
    assert!(config.contains("github = [\"r-lib/pak@v0.9.0\"]"));
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
    assert!(config.contains("dependencies = [\"pandas>=2,<3\", \"requests[socks]==2.32.3\"]"));
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
            .replace("name = \"test-project\"", "name = \"changed\""),
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
fn add_preserves_existing_trust_for_the_suggested_plain_up() {
    let fixture = backend_fixture();
    let home = fixture.engine.parent().unwrap().join("add-trust-home");

    untrusted_dual_command(&fixture, &home)
        .args(["--trust-project", "up"])
        .assert()
        .success();

    untrusted_dual_command(&fixture, &home)
        .args(["add", "py", "certifi"])
        .assert()
        .success();

    untrusted_dual_command(&fixture, &home)
        .env("DUAL_ENGINE_FAIL_LOCKED", "1")
        .arg("up")
        .assert()
        .failure()
        .stderr(predicate::str::contains("dual up --refresh"))
        .stderr(predicate::str::contains("project is not trusted").not());
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
        .env(
            "DUAL_HOME",
            fixture.engine.parent().unwrap().join("nested-shell-home"),
        )
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
            .replace("cran = [\"targets@1.11.4\"]", "cran = []")
            .replacen("bioc = []", "bioc = [\"DESeq2\"]", 1),
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
        .stdout(
            predicate::str::contains("analysis complete")
                .and(predicate::str::contains("Pixi task").not()),
        );
}

#[cfg(unix)]
#[test]
fn run_executes_task_dependencies_first() {
    let fixture = backend_fixture();
    fs::write(
        fixture.project.path().join("dual.toml"),
        fs::read_to_string(fixture.project.path().join("dual.toml"))
            .unwrap()
            .replace(
                "[tasks]\n",
                "[tasks]\nprepare = \"python prepare.py\"\nanalysis = { cmd = \"python analysis.py\", deps = [\"prepare\"] }\n",
            ),
    )
    .unwrap();
    write_ready_environment(fixture.project.path());
    write_test_lock(fixture.project.path(), "lock");

    dual_command(&fixture)
        .args(["run", "analysis"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Running task `prepare`")
                .and(predicate::str::contains("Running task `analysis`")),
        );

    let log = fs::read_to_string(&fixture.log).unwrap();
    let prepare = log.find(" prepare\n").unwrap();
    let analysis = log.find(" analysis\n").unwrap();
    assert!(prepare < analysis, "{log}");
}

#[cfg(unix)]
#[test]
fn run_script_prepares_environment_executes_and_records_lock_metadata() {
    let fixture = backend_fixture();
    fs::write(
        fixture.project.path().join("analysis.py"),
        "# /// script\n# requires-python = \">=3.12\"\n# dependencies = [\"rich\"]\n# ///\nprint('ok')\n",
    )
    .unwrap();

    dual_command(&fixture)
        .args(["run", "analysis.py", "--no-install"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("dual sync --script"));
    dual_command(&fixture)
        .args(["run", "analysis.py"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Running"));

    let calls = fs::read_to_string(&fixture.log).unwrap();
    assert!(calls.contains("__dual_script"));
    let script_state = fs::read_dir(fixture.project.path().join(".dual/scripts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(script_state.join("dual.lock")).unwrap()).unwrap();
    assert_eq!(lock["metadata"]["python"]["requested"], ">=3.12");
    assert_eq!(lock["metadata"]["python"]["dependencies"][0], "rich");
    assert!(lock["metadata"]["timestamp"].is_number());
}

#[cfg(unix)]
#[test]
fn task_results_do_not_invalidate_trust() {
    let fixture = backend_fixture();
    let home = fixture.engine.parent().unwrap().join("results-trust-home");
    configure_analysis_task(fixture.project.path());

    untrusted_dual_command(&fixture, &home)
        .args(["--trust-project", "up"])
        .assert()
        .success();

    fs::create_dir_all(fixture.project.path().join("results")).unwrap();
    fs::write(
        fixture.project.path().join("results/output.json"),
        "{\"ok\":true}\n",
    )
    .unwrap();

    untrusted_dual_command(&fixture, &home)
        .args(["run", "analysis"])
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn task_source_mutations_are_rejected_after_execution() {
    let fixture = backend_fixture();
    let home = fixture.engine.parent().unwrap().join("task-mutation-home");
    configure_analysis_task(fixture.project.path());

    untrusted_dual_command(&fixture, &home)
        .args(["--trust-project", "up"])
        .assert()
        .success();

    untrusted_dual_command(&fixture, &home)
        .env("DUAL_ENGINE_MUTATE_TASK", "1")
        .args(["run", "analysis"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Project files changed while code was executing",
        ));
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

#[cfg(unix)]
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

#[cfg(unix)]
fn configure_analysis_task(project: &std::path::Path) {
    let config_path = project.join("dual.toml");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path)
            .unwrap()
            .replace("[tasks]\n", "[tasks]\nanalysis = \"python script.py\"\n"),
    )
    .unwrap();
}

#[cfg(unix)]
fn write_ready_environment(project: &std::path::Path) {
    let config = dual::config::Config::load(project).unwrap();
    let manifest = dual::backend::generate_manifest(&config, project).unwrap();
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
  if [ "${DUAL_ENGINE_MUTATE_TASK:-0}" = "1" ]; then
    printf '%s\n' '# changed by task' >> dual.toml
  fi
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

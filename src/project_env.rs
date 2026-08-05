use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::security;

pub const PROJECT_ENV_FILE: &str = ".env";
pub const MAX_PROJECT_ENV_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectEnvironment {
    variables: BTreeMap<String, String>,
}

impl ProjectEnvironment {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(PROJECT_ENV_FILE);
        security::reject_symlink_if_present(&path, PROJECT_ENV_FILE)?;
        if !path.try_exists()? {
            return Ok(Self::default());
        }
        let contents = security::read_text_file(&path, MAX_PROJECT_ENV_BYTES, PROJECT_ENV_FILE)?;

        let mut variables = BTreeMap::new();
        for entry in dotenvy::from_read_iter(contents.as_bytes()) {
            let (name, value) = entry.map_err(|_| {
                anyhow::anyhow!(
                    "could not parse {}; check its dotenv syntax",
                    path.display()
                )
            })?;
            validate_name(&name)?;
            // Match dotenvy's non-overriding load behavior: the first value in
            // the file wins, and the invoking environment wins later.
            variables.entry(name).or_insert(value);
        }
        Ok(Self { variables })
    }

    pub fn apply_to_command(&self, command: &mut Command) {
        for (name, value) in &self.variables {
            if std::env::var_os(name).is_none() && !command_defines(command, name) {
                command.env(name, value);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }
}

fn validate_name(name: &str) -> Result<()> {
    let canonical = name.to_ascii_uppercase();
    if canonical.starts_with("DUAL_") {
        anyhow::bail!("{PROJECT_ENV_FILE} cannot set reserved Dual variable `{name}`");
    }
    if ["PIXI_", "CONDA_", "MAMBA_", "RATTLER_"]
        .iter()
        .any(|prefix| canonical.starts_with(prefix))
    {
        anyhow::bail!("{PROJECT_ENV_FILE} cannot set reserved engine variable `{name}`");
    }
    if ["LD_", "DYLD_"]
        .iter()
        .any(|prefix| canonical.starts_with(prefix))
    {
        anyhow::bail!("{PROJECT_ENV_FILE} cannot set reserved loader variable `{name}`");
    }
    if matches!(
        canonical.as_str(),
        "PATH"
            | "HOME"
            | "USERPROFILE"
            | "LOCALAPPDATA"
            | "APPDATA"
            | "SHELL"
            | "COMSPEC"
            | "PATHEXT"
            | "BASH_ENV"
            | "ENV"
            | "ZDOTDIR"
            | "PROMPT_COMMAND"
            | "PYTHONHOME"
            | "PYTHONPATH"
            | "PYTHONSTARTUP"
            | "PYTHONUSERBASE"
            | "VIRTUAL_ENV"
            | "R_HOME"
            | "R_USER"
            | "R_LIBS"
            | "R_LIBS_USER"
            | "R_LIBS_SITE"
            | "R_ENVIRON"
            | "R_ENVIRON_USER"
            | "R_PROFILE"
            | "R_PROFILE_USER"
            | "RETICULATE_PYTHON"
    ) {
        anyhow::bail!("{PROJECT_ENV_FILE} cannot set reserved process variable `{name}`");
    }
    Ok(())
}

fn command_defines(command: &Command, name: &str) -> bool {
    command
        .get_envs()
        .any(|(configured, _)| env_names_equal(configured, OsStr::new(name)))
}

#[cfg(windows)]
fn env_names_equal(left: &OsStr, right: &OsStr) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

#[cfg(not(windows))]
fn env_names_equal(left: &OsStr, right: &OsStr) -> bool {
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_an_empty_environment() {
        let directory = tempfile::tempdir().unwrap();
        assert!(ProjectEnvironment::load(directory.path())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn parses_dotenv_features_without_modifying_the_process() {
        let directory = tempfile::tempdir().unwrap();
        let host_name = "PROJECT_ENV_DOTENVY_TEST_HOST_7F3A";
        let process_value = std::env::var_os(host_name);
        std::fs::write(
            directory.path().join(PROJECT_ENV_FILE),
            format!(
                "{host_name}=example.test\nURL=\"https://${{{host_name}}}/api\"\nMULTILINE=\"first\\nsecond\"\n{host_name}=ignored.test\n"
            ),
        )
        .unwrap();

        let environment = ProjectEnvironment::load(directory.path()).unwrap();
        assert_eq!(environment.variables[host_name], "example.test");
        let expected_url = process_value
            .as_ref()
            .map(|value| format!("https://{}/api", value.to_string_lossy()))
            .unwrap_or_else(|| "https://example.test/api".into());
        assert_eq!(environment.variables["URL"], expected_url);
        assert_eq!(environment.variables["MULTILINE"], "first\nsecond");
        assert_eq!(std::env::var_os(host_name), process_value);
    }

    #[test]
    fn applies_values_without_overwriting_command_values() {
        let environment = ProjectEnvironment {
            variables: BTreeMap::from([
                ("PROJECT_ENV_FIRST".into(), "file".into()),
                ("PROJECT_ENV_SECOND".into(), "file".into()),
            ]),
        };
        let mut command = Command::new("program");
        command.env("PROJECT_ENV_FIRST", "command");
        environment.apply_to_command(&mut command);

        let configured = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(configured["PROJECT_ENV_FIRST"].as_deref(), Some("command"));
        assert_eq!(configured["PROJECT_ENV_SECOND"].as_deref(), Some("file"));
    }

    #[test]
    fn rejects_control_plane_and_runtime_injection_variables() {
        for name in [
            "DUAL_TRUST_PROJECT",
            "dual_engine_path",
            "PATH",
            "PythonPath",
            "R_PROFILE_USER",
            "LD_PRELOAD",
            "PIXI_HOME",
        ] {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(
                directory.path().join(PROJECT_ENV_FILE),
                format!("{name}=unsafe\n"),
            )
            .unwrap();
            let error = ProjectEnvironment::load(directory.path())
                .unwrap_err()
                .to_string();
            assert!(error.contains(name));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_environment_file() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), directory.path().join(PROJECT_ENV_FILE)).unwrap();
        let error = ProjectEnvironment::load(directory.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("symbolic link"));
    }

    #[test]
    fn rejects_malformed_and_oversized_environment_files() {
        let malformed = tempfile::tempdir().unwrap();
        let secret = "must-not-appear-in-errors";
        std::fs::write(
            malformed.path().join(PROJECT_ENV_FILE),
            format!("UNTERMINATED=\"{secret}\n"),
        )
        .unwrap();
        let error = ProjectEnvironment::load(malformed.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("could not parse"));
        assert!(!error.contains(secret));

        let oversized = tempfile::tempdir().unwrap();
        std::fs::write(
            oversized.path().join(PROJECT_ENV_FILE),
            vec![b'x'; MAX_PROJECT_ENV_BYTES as usize + 1],
        )
        .unwrap();
        let error = ProjectEnvironment::load(oversized.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("safety limit"));
    }
}

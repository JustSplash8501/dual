use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Value};

use crate::errors::DualError;
use crate::security::{self, MAX_CONFIG_BYTES};

pub const DEFAULT_CONFIG: &str = r#"[project]
name = "my-project"

[r]
version = "4.5"
packages = []

[python]
version = "3.12"
packages = []

[tasks]
# Example:
# analysis = "Rscript scripts/analysis.R"
# model = "python scripts/model.py"
# report = "quarto render manuscript.qmd"
"#;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub project: ProjectConfig,
    pub r: LanguageConfig,
    pub python: LanguageConfig,
    #[serde(default)]
    pub tasks: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LanguageConfig {
    pub version: String,
    #[serde(default)]
    pub packages: Vec<String>,
}

impl Config {
    pub fn path(root: &Path) -> PathBuf {
        root.join("dual.toml")
    }

    pub fn load(root: &Path) -> Result<Self> {
        Self::from_path(&Self::path(root))
    }

    pub fn find_root(start: &Path) -> Result<PathBuf> {
        for directory in start.ancestors() {
            if Self::path(directory).is_file() {
                return Ok(directory.to_path_buf());
            }
        }
        Err(DualError::MissingConfig(start.display().to_string()).into())
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(DualError::MissingConfig(
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .display()
                    .to_string(),
            )
            .into());
        }

        let contents = security::read_text_file(path, MAX_CONFIG_BYTES, "dual.toml")
            .with_context(|| format!("could not read {}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .map_err(|error| DualError::InvalidConfig(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.project.name.trim().is_empty() {
            return Err(DualError::InvalidConfig("project.name cannot be empty".into()).into());
        }
        reject_control_characters("project.name", &self.project.name)?;
        validate_language("r", &self.r)?;
        validate_language("python", &self.python)?;
        for (name, command) in &self.tasks {
            if name.trim().is_empty() || command.trim().is_empty() {
                return Err(DualError::InvalidConfig(
                    "task names and commands cannot be empty".into(),
                )
                .into());
            }
            if name.starts_with('-') {
                return Err(
                    DualError::InvalidConfig("task names cannot start with `-`".into()).into(),
                );
            }
            reject_control_characters("task name", name)?;
            reject_control_characters("task command", command)?;
        }
        Ok(())
    }

    pub fn add_packages(path: &Path, section: &str, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            anyhow::bail!("provide at least one package name");
        }
        for package in packages {
            let valid = if section == "r" {
                valid_r_package_reference(package)
            } else {
                parse_python_requirement(package).is_ok()
            };
            if !valid {
                anyhow::bail!("invalid package name: {package:?}");
            }
        }

        // Validate the current file before editing it.
        Self::from_path(path)?;
        let contents = security::read_text_file(path, MAX_CONFIG_BYTES, "dual.toml")?;
        let mut document = contents
            .parse::<DocumentMut>()
            .map_err(|error| DualError::InvalidConfig(error.to_string()))?;

        let array_item = document
            .get_mut(section)
            .and_then(Item::as_table_mut)
            .and_then(|table| table.get_mut("packages"))
            .ok_or_else(|| DualError::InvalidConfig(format!("[{section}].packages is required")))?;

        let array = array_item.as_array_mut().ok_or_else(|| {
            DualError::InvalidConfig(format!("[{section}].packages must be an array"))
        })?;

        let mut existing = array
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();

        for package in packages {
            if !existing.iter().any(|item| item == package) {
                existing.push(package.clone());
            }
        }

        let mut replacement = Array::new();
        for package in existing {
            replacement.push(package);
        }
        *array = replacement;
        security::write_file_atomic(path, document.to_string().as_bytes(), "dual.toml")?;

        // Ensure the edit still yields a valid typed configuration.
        Self::from_path(path)?;
        Ok(())
    }

    pub fn remove_packages(path: &Path, section: &str, packages: &[String]) -> Result<usize> {
        if packages.is_empty() {
            anyhow::bail!("provide at least one package name");
        }
        Self::from_path(path)?;
        let contents = security::read_text_file(path, MAX_CONFIG_BYTES, "dual.toml")?;
        let mut document = contents
            .parse::<DocumentMut>()
            .map_err(|error| DualError::InvalidConfig(error.to_string()))?;
        let array = document
            .get_mut(section)
            .and_then(Item::as_table_mut)
            .and_then(|table| table.get_mut("packages"))
            .and_then(Item::as_array_mut)
            .ok_or_else(|| {
                DualError::InvalidConfig(format!("[{section}].packages must be an array"))
            })?;
        let existing = array
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let retained = existing
            .iter()
            .filter(|package| !packages.contains(package))
            .cloned()
            .collect::<Vec<_>>();
        let removed = existing.len() - retained.len();
        let mut replacement = Array::new();
        for package in retained {
            replacement.push(package);
        }
        *array = replacement;
        security::write_file_atomic(path, document.to_string().as_bytes(), "dual.toml")?;
        Self::from_path(path)?;
        Ok(removed)
    }
}

fn validate_language(name: &str, language: &LanguageConfig) -> Result<()> {
    if language.version.trim().is_empty() {
        return Err(DualError::InvalidConfig(format!("{name}.version cannot be empty")).into());
    }
    reject_control_characters(&format!("{name}.version"), &language.version)?;
    if !valid_version_specifier(&language.version) {
        return Err(DualError::InvalidConfig(format!(
            "{name}.version contains unsupported characters"
        ))
        .into());
    }
    if let Some(package) = language.packages.iter().find(|package| {
        if name == "r" {
            !valid_r_package_reference(package)
        } else {
            parse_python_requirement(package).is_err()
        }
    }) {
        return Err(DualError::InvalidConfig(format!(
            "{name}.packages contains an invalid package name: {package:?}"
        ))
        .into());
    }
    Ok(())
}

fn reject_control_characters(field: &str, value: &str) -> Result<()> {
    if security::contains_control_characters(value) {
        return Err(
            DualError::InvalidConfig(format!("{field} cannot contain control characters")).into(),
        );
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonRequirement {
    pub name: String,
    pub extras: Vec<String>,
    pub version: String,
}

pub fn parse_python_requirement(requirement: &str) -> Result<PythonRequirement> {
    let value = requirement.trim();
    if value.is_empty()
        || value.starts_with('-')
        || value
            .chars()
            .any(|character| matches!(character, '\'' | '"' | '\n' | '\r' | ';' | '@'))
    {
        anyhow::bail!("unsupported Python requirement: {requirement:?}");
    }

    let split_at = value
        .char_indices()
        .find(|(_, character)| matches!(character, '<' | '>' | '=' | '!' | '~'))
        .map(|(index, _)| index)
        .unwrap_or(value.len());
    let (name_and_extras, version) = value.split_at(split_at);
    let version = if version.is_empty() { "*" } else { version };

    let (name, extras) = if let Some(open) = name_and_extras.find('[') {
        if !name_and_extras.ends_with(']') {
            anyhow::bail!("malformed Python extras: {requirement:?}");
        }
        let name = &name_and_extras[..open];
        let extras = &name_and_extras[open + 1..name_and_extras.len() - 1];
        let extras = extras
            .split(',')
            .map(str::trim)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if extras.is_empty() || extras.iter().any(|extra| !valid_distribution_name(extra)) {
            anyhow::bail!("malformed Python extras: {requirement:?}");
        }
        (name, extras)
    } else {
        (name_and_extras, Vec::new())
    };

    if !valid_distribution_name(name) || !valid_version_specifier(version) {
        anyhow::bail!("unsupported Python requirement: {requirement:?}");
    }

    Ok(PythonRequirement {
        name: name.to_owned(),
        extras,
        version: version.to_owned(),
    })
}

fn valid_distribution_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn valid_version_specifier(value: &str) -> bool {
    value == "*"
        || (!value.chars().any(char::is_whitespace)
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(
                        character,
                        '<' | '>' | '=' | '!' | '~' | '.' | ',' | '*' | '+' | '-'
                    )
            }))
}

fn valid_r_package_reference(package: &str) -> bool {
    if !valid_package_name(package) {
        return false;
    }
    let reference = package
        .split_once('=')
        .map(|(name, reference)| {
            if name.trim().is_empty() {
                return "";
            }
            reference
        })
        .unwrap_or(package);
    let Some((source, target)) = reference.split_once("::") else {
        return !package.contains('=');
    };
    if target.is_empty() || !matches!(source, "cran" | "bioc" | "github") {
        return false;
    }
    if source == "github" {
        let repository = target.split(['@', '#', '?']).next().unwrap_or_default();
        let mut parts = repository.split('/');
        return parts.next().is_some_and(|part| !part.is_empty())
            && parts.next().is_some_and(|part| !part.is_empty());
    }
    !target.starts_with(['@', '?']) && !target.contains('/')
}

fn valid_package_name(package: &str) -> bool {
    !package.trim().is_empty()
        && !package.starts_with('-')
        && package.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '.' | '_' | '-' | ':' | '/' | '@' | '#' | '?' | '=' | '&' | '+' | '%' | '~'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_config() {
        let config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        config.validate().unwrap();
        assert_eq!(config.r.version, "4.5");
        assert_eq!(config.python.version, "3.12");
    }

    #[test]
    fn rejects_unknown_fields() {
        let invalid = DEFAULT_CONFIG.replace(
            "name = \"my-project\"",
            "name = \"my-project\"\nunknown = true",
        );
        assert!(toml::from_str::<Config>(&invalid).is_err());
    }

    #[test]
    fn rejects_empty_project_name() {
        let invalid = DEFAULT_CONFIG.replace("my-project", "");
        let config: Config = toml::from_str(&invalid).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_control_characters() {
        let mut config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        config.project.name = "unsafe\u{1b}[2J".into();
        assert!(config.validate().is_err());

        let mut config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        config.project.name = "safe\u{202e}gpj.exe".into();
        assert!(config.validate().is_err());

        let mut config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        config
            .tasks
            .insert("unsafe".into(), "echo\u{7}danger".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_oversized_config_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dual.toml");
        fs::write(&path, vec![b' '; security::MAX_CONFIG_BYTES as usize + 1]).unwrap();
        let error = Config::from_path(&path).unwrap_err();
        assert!(format!("{error:#}").contains("safety limit"));
    }

    #[test]
    fn accepts_supported_r_package_sources() {
        let mut config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        config.r.packages = vec![
            "cran::targets@1.11.4".into(),
            "bioc::DESeq2".into(),
            "github::r-lib/pak@v0.9.0".into(),
            "actual=github::owner/repository@abc123".into(),
        ];
        config.validate().unwrap();
    }

    #[test]
    fn rejects_malformed_r_package_sources() {
        for package in [
            "cran::",
            "bioc::package/name",
            "github::repository",
            "unknown::package",
            "=github::owner/repository",
        ] {
            let mut config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
            config.r.packages = vec![package.into()];
            assert!(config.validate().is_err(), "{package} should be invalid");
        }
    }

    #[test]
    fn parses_python_versions_and_extras() {
        assert_eq!(
            parse_python_requirement("pandas>=2,<3").unwrap(),
            PythonRequirement {
                name: "pandas".into(),
                extras: vec![],
                version: ">=2,<3".into(),
            }
        );
        assert_eq!(
            parse_python_requirement("requests[socks,security]==2.32.3").unwrap(),
            PythonRequirement {
                name: "requests".into(),
                extras: vec!["socks".into(), "security".into()],
                version: "==2.32.3".into(),
            }
        );
    }

    #[test]
    fn finds_project_root_from_a_subdirectory() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("dual.toml"), DEFAULT_CONFIG).unwrap();
        let nested = directory.path().join("scripts/nested");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(Config::find_root(&nested).unwrap(), directory.path());
    }
}

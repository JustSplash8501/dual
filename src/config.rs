use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml_edit::{Array, DocumentMut, Item, Value};

use crate::errors::DualError;
use crate::metadata::{self, ScriptKind, ScriptMetadata};
use crate::security::{self, MAX_CONFIG_BYTES};

pub const DEFAULT_CONFIG: &str = r#"[project]
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
# Example:
# analysis = "Rscript scripts/analysis.R"
# model = "python scripts/model.py"
# report = "quarto render manuscript.qmd"
"#;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub project: ProjectConfig,
    pub r: RConfig,
    pub python: PythonConfig,
    #[serde(default)]
    pub quarto: QuartoConfig,
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
pub struct PythonConfig {
    #[serde(skip, default = "enabled_by_default")]
    pub enabled: bool,
    pub version: String,
    #[serde(default, rename = "dependencies", alias = "packages")]
    pub packages: Vec<String>,
    #[serde(default)]
    pub index: Vec<PackageIndex>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageIndex {
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RConfig {
    pub enabled: bool,
    pub version: String,
    pub packages: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuartoConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRConfig {
    version: String,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    cran: Vec<String>,
    #[serde(default)]
    bioc: Vec<String>,
    #[serde(default)]
    github: Vec<String>,
}

#[derive(Serialize)]
struct SerializedRConfig<'a> {
    version: &'a str,
    cran: Vec<&'a str>,
    bioc: Vec<&'a str>,
    github: Vec<&'a str>,
}

impl<'de> Deserialize<'de> for RConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRConfig::deserialize(deserializer)?;
        let mut packages = raw.packages;
        packages.extend(
            raw.cran
                .into_iter()
                .map(|package| format!("cran::{package}")),
        );
        packages.extend(
            raw.bioc
                .into_iter()
                .map(|package| format!("bioc::{package}")),
        );
        packages.extend(
            raw.github
                .into_iter()
                .map(|package| format!("github::{package}")),
        );
        deduplicate(&mut packages);
        Ok(Self {
            enabled: true,
            version: raw.version,
            packages,
        })
    }
}

impl Serialize for RConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut cran = Vec::new();
        let mut bioc = Vec::new();
        let mut github = Vec::new();
        for package in &self.packages {
            if let Some(package) = package.strip_prefix("bioc::") {
                bioc.push(package);
            } else if let Some(package) = package.strip_prefix("github::") {
                github.push(package);
            } else if let Some(package) = package.strip_prefix("cran::") {
                cran.push(package);
            } else {
                cran.push(package.as_str());
            }
        }
        SerializedRConfig {
            version: &self.version,
            cran,
            bioc,
            github,
        }
        .serialize(serializer)
    }
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

    pub fn find_root_optional(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .find(|directory| Self::path(directory).is_file())
            .map(Path::to_path_buf)
    }

    pub fn for_script(path: &Path) -> Result<EffectiveConfig> {
        let path = metadata::absolute_path(path)?;
        if !path.is_file() {
            anyhow::bail!("script was not found: {}", path.display());
        }
        let kind = ScriptKind::from_path(&path)?;
        let directory = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("script has no parent directory"))?;
        let project_root = Self::find_root_optional(directory);
        let mut config = if let Some(root) = &project_root {
            Self::load(root)?
        } else {
            Self::empty("dual-script")
        };
        let inline = metadata::read(&path)?;
        if let Some(inline) = &inline {
            config.merge_script_metadata(&inline.metadata);
        }
        if project_root.is_none() {
            match kind {
                ScriptKind::Python => config.python.enabled = true,
                ScriptKind::R => config.r.enabled = true,
                ScriptKind::Quarto | ScriptKind::RMarkdown if inline.is_none() => {
                    config.python.enabled = true;
                    config.r.enabled = true;
                }
                ScriptKind::Quarto | ScriptKind::RMarkdown => {}
            }
        }
        if matches!(kind, ScriptKind::Quarto | ScriptKind::RMarkdown) {
            config.quarto.enabled = true;
        }
        config.validate()?;
        let source = match (project_root.is_some(), inline.is_some()) {
            (true, true) => MetadataSource::ProjectAndInline,
            (true, false) => MetadataSource::Project,
            (false, true) => MetadataSource::Inline,
            (false, false) => MetadataSource::Defaults,
        };
        Ok(EffectiveConfig {
            root: project_root.unwrap_or_else(|| directory.to_owned()),
            script: path,
            kind,
            config,
            source,
        })
    }

    pub fn empty(project_name: &str) -> Self {
        Self {
            project: ProjectConfig {
                name: project_name.to_owned(),
            },
            r: RConfig {
                enabled: false,
                version: "4.5".to_owned(),
                packages: Vec::new(),
            },
            python: PythonConfig {
                enabled: false,
                version: "3.12".to_owned(),
                packages: Vec::new(),
                index: Vec::new(),
            },
            quarto: QuartoConfig::default(),
            tasks: BTreeMap::new(),
        }
    }

    pub fn merge_script_metadata(&mut self, metadata: &ScriptMetadata) {
        if let Some(version) = &metadata.python_version {
            self.python.enabled = true;
            self.python.version = version.clone();
        }
        if let Some(version) = &metadata.r_version {
            self.r.enabled = true;
            self.r.version = version.clone();
        }
        if !metadata.python_dependencies.is_empty() || !metadata.python_indexes.is_empty() {
            self.python.enabled = true;
        }
        if !metadata.cran.is_empty() || !metadata.bioc.is_empty() || !metadata.github.is_empty() {
            self.r.enabled = true;
        }
        self.python
            .packages
            .extend(metadata.python_dependencies.iter().cloned());
        self.python
            .index
            .extend(metadata.python_indexes.iter().cloned());
        self.r.packages.extend(
            metadata
                .cran
                .iter()
                .map(|package| format!("cran::{package}")),
        );
        self.r.packages.extend(
            metadata
                .bioc
                .iter()
                .map(|package| format!("bioc::{package}")),
        );
        self.r.packages.extend(
            metadata
                .github
                .iter()
                .map(|package| format!("github::{package}")),
        );
        deduplicate(&mut self.python.packages);
        deduplicate(&mut self.r.packages);
        let mut seen = std::collections::BTreeSet::new();
        self.python
            .index
            .retain(|index| seen.insert(index.url.clone()));
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
        validate_project_name(&self.project.name)
            .map_err(|error| DualError::InvalidConfig(error.to_string()))?;
        validate_r(&self.r)?;
        validate_python(&self.python)?;
        for index in &self.python.index {
            validate_index_url(&index.url)?;
        }
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

        let table = document
            .get_mut(section)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| DualError::InvalidConfig(format!("[{section}] is required")))?;

        if section == "python" {
            let key = if table.contains_key("dependencies") {
                "dependencies"
            } else {
                "packages"
            };
            append_to_array(table, key, packages.iter().map(String::as_str))?;
        } else {
            for package in packages {
                let (key, value) = project_r_package(package);
                append_to_array(table, key, std::iter::once(value))?;
            }
        }
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
        let table = document
            .get_mut(section)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| DualError::InvalidConfig(format!("[{section}] is required")))?;
        let keys = if section == "python" {
            vec!["dependencies", "packages"]
        } else {
            vec!["cran", "bioc", "github", "packages"]
        };
        let mut removed = 0;
        for key in keys {
            let Some(array) = table.get_mut(key).and_then(Item::as_array_mut) else {
                continue;
            };
            let existing = array
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let retained = existing
                .iter()
                .filter(|package| {
                    let canonical = canonical_project_package(section, key, package);
                    !packages
                        .iter()
                        .any(|requested| requested == *package || requested == &canonical)
                })
                .cloned()
                .collect::<Vec<_>>();
            removed += existing.len() - retained.len();
            let mut replacement = Array::new();
            for package in retained {
                replacement.push(package);
            }
            *array = replacement;
        }
        security::write_file_atomic(path, document.to_string().as_bytes(), "dual.toml")?;
        Self::from_path(path)?;
        Ok(removed)
    }
}

fn append_to_array<'a>(
    table: &mut toml_edit::Table,
    key: &str,
    values: impl Iterator<Item = &'a str>,
) -> Result<()> {
    if !table.contains_key(key) {
        table.insert(key, Item::Value(Value::Array(Array::new())));
    }
    let array = table
        .get_mut(key)
        .and_then(Item::as_array_mut)
        .ok_or_else(|| DualError::InvalidConfig(format!("array expected for `{key}`")))?;
    let mut existing = array
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for value in values {
        if !existing.iter().any(|item| item == value) {
            existing.push(value.to_owned());
        }
    }
    let mut replacement = Array::new();
    for value in existing {
        replacement.push(value);
    }
    *array = replacement;
    Ok(())
}

fn project_r_package(package: &str) -> (&str, &str) {
    if let Some(package) = package.strip_prefix("cran::") {
        ("cran", package)
    } else if let Some(package) = package.strip_prefix("bioc::") {
        ("bioc", package)
    } else if let Some(package) = package.strip_prefix("github::") {
        ("github", package)
    } else {
        ("packages", package)
    }
}

fn canonical_project_package(section: &str, key: &str, package: &str) -> String {
    if section == "python" || key == "packages" || key == "cran" {
        package.to_owned()
    } else {
        format!("{key}::{package}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataSource {
    Project,
    Inline,
    ProjectAndInline,
    Defaults,
}

impl std::fmt::Display for MetadataSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Project => formatter.write_str("project dual.toml"),
            Self::Inline => formatter.write_str("inline script metadata"),
            Self::ProjectAndInline => {
                formatter.write_str("project dual.toml and inline script metadata")
            }
            Self::Defaults => formatter.write_str("defaults (no dependency metadata found)"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EffectiveConfig {
    pub root: PathBuf,
    pub script: PathBuf,
    pub kind: ScriptKind,
    pub config: Config,
    pub source: MetadataSource,
}

pub fn validate_project_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && name
            .chars()
            .last()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if !valid {
        anyhow::bail!(
            "project name must start and end with a letter or number and contain only ASCII letters, numbers, `-`, or `_`"
        );
    }
    Ok(())
}

fn validate_python(language: &PythonConfig) -> Result<()> {
    let name = "python";
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

fn validate_r(language: &RConfig) -> Result<()> {
    if language.version.trim().is_empty() {
        return Err(DualError::InvalidConfig("r.version cannot be empty".into()).into());
    }
    reject_control_characters("r.version", &language.version)?;
    if !valid_version_specifier(&language.version) {
        return Err(
            DualError::InvalidConfig("r.version contains unsupported characters".into()).into(),
        );
    }
    if let Some(package) = language
        .packages
        .iter()
        .find(|package| !valid_r_package_reference(package))
    {
        return Err(DualError::InvalidConfig(format!(
            "r packages contain an invalid package name: {package:?}"
        ))
        .into());
    }
    Ok(())
}

pub fn validate_index_url(url: &str) -> Result<()> {
    let value = url.trim();
    if security::contains_control_characters(value)
        || !(value.starts_with("https://") || value.starts_with("http://"))
        || value.chars().any(char::is_whitespace)
    {
        anyhow::bail!("package index URL must be an http:// or https:// URL without whitespace");
    }
    Ok(())
}

fn deduplicate(values: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn enabled_by_default() -> bool {
    true
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

pub fn valid_version_specifier(value: &str) -> bool {
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

pub fn valid_r_package_reference(package: &str) -> bool {
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
    fn parses_modern_project_dependency_sections() {
        let config: Config = toml::from_str(
            r#"[project]
name = "example"

[python]
version = ">=3.12"
dependencies = ["pandas"]

[[python.index]]
url = "https://example.com/simple"

[r]
version = ">=4.4"
cran = ["tidyverse"]
bioc = ["DESeq2"]
github = ["hadley/emo"]

[quarto]
enabled = true
"#,
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.python.packages, ["pandas"]);
        assert_eq!(config.r.packages[0], "cran::tidyverse");
        assert!(config.r.packages.contains(&"bioc::DESeq2".to_owned()));
        assert!(config.r.packages.contains(&"github::hadley/emo".to_owned()));
        assert!(config.quarto.enabled);
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
    fn rejects_unsafe_project_names() {
        for name in ["has spaces", "café", "-leading", "trailing-", "dot.name"] {
            let mut config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
            config.project.name = name.into();
            assert!(config.validate().is_err(), "accepted {name:?}");
        }
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

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml_edit::{Array, DocumentMut, Item, Value};

use crate::errors::DualError;
use crate::metadata::{self, ScriptKind, ScriptMetadata};
use crate::security::{self, MAX_CONFIG_BYTES};

pub const DEFAULT_R_VERSION: &str = "4.5";
pub const DEFAULT_PYTHON_VERSION: &str = "3.12";

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
    #[serde(default, skip_serializing_if = "r_disabled")]
    pub r: RConfig,
    #[serde(default, skip_serializing_if = "python_disabled")]
    pub python: PythonConfig,
    #[serde(default)]
    pub quarto: QuartoConfig,
    #[serde(default)]
    pub tasks: BTreeMap<String, TaskConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnableLanguageResult {
    pub changed: bool,
    pub previous_version: Option<String>,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisableLanguageResult {
    pub changed: bool,
    pub removed_packages: usize,
    pub removed_indexes: usize,
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

impl Default for PythonConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            version: DEFAULT_PYTHON_VERSION.to_owned(),
            packages: Vec::new(),
            index: Vec::new(),
        }
    }
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

impl Default for RConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            version: DEFAULT_R_VERSION.to_owned(),
            packages: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuartoConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TaskConfig {
    Command(String),
    Detailed(TaskDetails),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskDetails {
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
}

impl TaskConfig {
    pub fn simple(command: impl Into<String>) -> Self {
        Self::Command(command.into())
    }

    pub fn command(&self) -> &str {
        match self {
            Self::Command(command) => command,
            Self::Detailed(details) => &details.cmd,
        }
    }

    pub fn deps(&self) -> &[String] {
        match self {
            Self::Command(_) => &[],
            Self::Detailed(details) => &details.deps,
        }
    }
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
            r: RConfig::default(),
            python: PythonConfig::default(),
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
        if !self.r.enabled && !self.python.enabled && !self.quarto.enabled {
            return Err(DualError::InvalidConfig(
                "at least one of [r], [python], or enabled [quarto] is required".into(),
            )
            .into());
        }
        validate_r(&self.r)?;
        validate_python(&self.python)?;
        for index in &self.python.index {
            validate_index_url(&index.url)?;
        }
        for (name, task) in &self.tasks {
            if name.trim().is_empty() || task.command().trim().is_empty() {
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
            reject_control_characters("task command", task.command())?;
            for dependency in task.deps() {
                if dependency.trim().is_empty() {
                    return Err(DualError::InvalidConfig(
                        "task dependencies cannot be empty".into(),
                    )
                    .into());
                }
                reject_control_characters("task dependency", dependency)?;
            }
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

        ensure_language_table(&mut document, section)?;
        let table = document[section]
            .as_table_mut()
            .expect("language section was created as a table");

        if section == "python" {
            let key = if table.contains_key("dependencies") {
                "dependencies"
            } else {
                "packages"
            };
            append_to_array(table, key, packages.iter().map(String::as_str))?;
        } else {
            let mut grouped = BTreeMap::<&str, Vec<&str>>::new();
            for package in packages {
                let (key, value) = project_r_package(package);
                grouped.entry(key).or_default().push(value);
            }
            for (key, values) in grouped {
                append_to_array(table, key, values.into_iter())?;
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
        let Some(table) = document.get_mut(section).and_then(Item::as_table_mut) else {
            return Ok(0);
        };
        let keys: &[&str] = if section == "python" {
            &["dependencies", "packages"]
        } else {
            &["cran", "bioc", "github", "packages"]
        };
        let requested = packages.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let mut removed = 0;
        for key in keys {
            let Some(array) = table.get_mut(key).and_then(Item::as_array_mut) else {
                continue;
            };
            let mut replacement = Array::new();
            let mut key_removed = 0;
            for package in array.iter().filter_map(Value::as_str) {
                let canonical = canonical_project_package(section, key, package);
                if requested.contains(package) || requested.contains(canonical.as_ref()) {
                    key_removed += 1;
                } else {
                    replacement.push(package);
                }
            }
            removed += key_removed;
            *array = replacement;
        }
        security::write_file_atomic(path, document.to_string().as_bytes(), "dual.toml")?;
        Self::from_path(path)?;
        Ok(removed)
    }

    pub fn enable_language(
        path: &Path,
        section: &str,
        version: Option<&str>,
    ) -> Result<EnableLanguageResult> {
        let current = Self::from_path(path)?;
        let previous_version = language_version(&current, section)?;
        let contents = security::read_text_file(path, MAX_CONFIG_BYTES, "dual.toml")?;
        let mut document = contents
            .parse::<DocumentMut>()
            .map_err(|error| DualError::InvalidConfig(error.to_string()))?;
        let was_present = document.contains_key(section);
        ensure_language_table(&mut document, section)?;

        let table = document[section]
            .as_table_mut()
            .expect("language section was created as a table");
        let selected_version = version
            .map(str::to_owned)
            .or_else(|| {
                table
                    .get("version")
                    .and_then(Item::as_str)
                    .map(str::to_owned)
            })
            .expect("language sections always contain a version");
        let version_changed =
            table.get("version").and_then(Item::as_str) != Some(selected_version.as_str());
        if version_changed {
            table.insert("version", toml_edit::value(&selected_version));
        }

        let changed = !was_present || version_changed;
        validate_document(&document)?;
        if changed {
            security::write_file_atomic(path, document.to_string().as_bytes(), "dual.toml")?;
        }
        Ok(EnableLanguageResult {
            changed,
            previous_version,
            version: selected_version,
        })
    }

    pub fn disable_language(
        path: &Path,
        section: &str,
        force: bool,
    ) -> Result<DisableLanguageResult> {
        let current = Self::from_path(path)?;
        if language_version(&current, section)?.is_none() {
            return Ok(DisableLanguageResult {
                changed: false,
                removed_packages: 0,
                removed_indexes: 0,
            });
        }

        let task_references = current
            .tasks
            .iter()
            .filter(|(_, task)| task_references_language(task.command(), section))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        if !task_references.is_empty() {
            anyhow::bail!(
                "cannot disable {} while tasks directly reference it: {}. Update those tasks first.",
                language_label(section)?,
                task_references.join(", ")
            );
        }

        let disabling_last_runtime = match section {
            "r" => !current.python.enabled,
            "python" => !current.r.enabled,
            _ => unreachable!("language_version validates the section"),
        };
        if disabling_last_runtime && !current.quarto.enabled {
            anyhow::bail!("cannot disable the last project runtime unless Quarto is enabled");
        }

        let (removed_packages, removed_indexes) = match section {
            "r" => (current.r.packages.len(), 0),
            "python" => (current.python.packages.len(), current.python.index.len()),
            _ => unreachable!("language_version validates the section"),
        };
        if !force && (removed_packages > 0 || removed_indexes > 0) {
            anyhow::bail!(
                "disabling {} would remove {} package(s) and {} package index(es). Remove them first or rerun with `--force`.",
                language_label(section)?,
                removed_packages,
                removed_indexes
            );
        }

        let contents = security::read_text_file(path, MAX_CONFIG_BYTES, "dual.toml")?;
        let mut document = contents
            .parse::<DocumentMut>()
            .map_err(|error| DualError::InvalidConfig(error.to_string()))?;
        document.remove(section);
        validate_document(&document)?;
        security::write_file_atomic(path, document.to_string().as_bytes(), "dual.toml")?;
        Ok(DisableLanguageResult {
            changed: true,
            removed_packages,
            removed_indexes,
        })
    }
}

fn validate_document(document: &DocumentMut) -> Result<Config> {
    let config: Config = toml::from_str(&document.to_string())
        .map_err(|error| DualError::InvalidConfig(error.to_string()))?;
    config.validate()?;
    Ok(config)
}

fn language_version(config: &Config, section: &str) -> Result<Option<String>> {
    match section {
        "r" => Ok(config.r.enabled.then(|| config.r.version.clone())),
        "python" => Ok(config.python.enabled.then(|| config.python.version.clone())),
        _ => Err(DualError::InvalidConfig(format!("unknown language section: {section}")).into()),
    }
}

fn language_label(section: &str) -> Result<&'static str> {
    match section {
        "r" => Ok("R"),
        "python" => Ok("Python"),
        _ => Err(DualError::InvalidConfig(format!("unknown language section: {section}")).into()),
    }
}

fn task_references_language(command: &str, section: &str) -> bool {
    let executable = command
        .split_whitespace()
        .next()
        .map(|part| part.trim_matches(['\'', '"']))
        .and_then(|part| Path::new(part).file_name())
        .and_then(|part| part.to_str())
        .map(|part| part.trim_end_matches(".exe").to_ascii_lowercase());
    let executable_matches = match (section, executable.as_deref()) {
        ("r", Some("r" | "rscript")) => true,
        ("python", Some("py" | "python" | "python3" | "pythonw")) => true,
        ("python", Some(executable)) => executable.strip_prefix("python").is_some_and(|suffix| {
            suffix
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        }),
        _ => false,
    };
    if executable_matches {
        return true;
    }

    let Some(script) = crate::platform::referenced_script(command) else {
        return false;
    };
    let script = script.to_string_lossy().to_ascii_lowercase();
    match section {
        "r" => script.ends_with(".r") || script.ends_with(".rmd") || script.ends_with(".qmd"),
        "python" => script.ends_with(".py") || script.ends_with(".qmd"),
        _ => false,
    }
}

pub(crate) fn ensure_language_table(document: &mut DocumentMut, section: &str) -> Result<()> {
    if document.contains_key(section) {
        if document[section].is_table() {
            return Ok(());
        }
        return Err(DualError::InvalidConfig(format!("[{section}] must be a table")).into());
    }

    let version = match section {
        "r" => DEFAULT_R_VERSION,
        "python" => DEFAULT_PYTHON_VERSION,
        _ => {
            return Err(
                DualError::InvalidConfig(format!("unknown language section: {section}")).into(),
            )
        }
    };
    let mut table = toml_edit::Table::new();
    table.insert("version", toml_edit::value(version));
    document.insert(section, Item::Table(table));
    Ok(())
}

fn r_disabled(config: &RConfig) -> bool {
    !config.enabled
}

fn python_disabled(config: &PythonConfig) -> bool {
    !config.enabled
}

pub fn starter_config(project_name: &str, python: Option<&str>, r: Option<&str>) -> Result<String> {
    validate_project_name(project_name)?;
    let explicit_languages = python.is_some() || r.is_some();
    let python = python.or((!explicit_languages).then_some(DEFAULT_PYTHON_VERSION));
    let r = r.or((!explicit_languages).then_some(DEFAULT_R_VERSION));

    let mut contents = format!("[project]\nname = \"{project_name}\"\n");
    if let Some(version) = r {
        contents.push_str(&format!(
            "\n[r]\nversion = \"{version}\"\ncran = []\nbioc = []\ngithub = []\n"
        ));
    }
    if let Some(version) = python {
        contents.push_str(&format!(
            "\n[python]\nversion = \"{version}\"\ndependencies = []\n"
        ));
    }
    contents.push_str(
        "\n[quarto]\nenabled = false\n\n[tasks]\n# Example:\n# analysis = \"Rscript scripts/analysis.R\"\n# model = \"python scripts/model.py\"\n# report = \"quarto render manuscript.qmd\"\n",
    );

    let config: Config =
        toml::from_str(&contents).map_err(|error| DualError::InvalidConfig(error.to_string()))?;
    config.validate()?;
    Ok(contents)
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
    let mut seen = existing.iter().cloned().collect::<BTreeSet<_>>();
    for value in values {
        let value = value.to_owned();
        if seen.insert(value.clone()) {
            existing.push(value);
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

fn canonical_project_package<'a>(section: &str, key: &str, package: &'a str) -> Cow<'a, str> {
    if section == "python" || key == "packages" || key == "cran" {
        Cow::Borrowed(package)
    } else {
        Cow::Owned(format!("{key}::{package}"))
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

pub fn valid_distribution_name_for_import(value: &str) -> bool {
    valid_distribution_name(value)
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
    fn accepts_single_language_projects_and_omits_disabled_sections() {
        let python: Config = toml::from_str(
            r#"[project]
name = "python-only"

[python]
version = "3.13"
dependencies = ["rich"]
"#,
        )
        .unwrap();
        python.validate().unwrap();
        assert!(!python.r.enabled);
        assert!(python.python.enabled);
        let rendered = toml::to_string(&python).unwrap();
        assert!(!rendered.contains("[r]"));
        assert!(rendered.contains("[python]"));

        let r: Config = toml::from_str(
            r#"[project]
name = "r-only"

[r]
version = "4.5"
cran = ["dplyr"]
"#,
        )
        .unwrap();
        r.validate().unwrap();
        assert!(r.r.enabled);
        assert!(!r.python.enabled);
        let rendered = toml::to_string(&r).unwrap();
        assert!(rendered.contains("[r]"));
        assert!(!rendered.contains("[python]"));
    }

    #[test]
    fn rejects_projects_without_a_language() {
        let config: Config = toml::from_str(
            r#"[project]
name = "empty"
"#,
        )
        .unwrap();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("at least one of [r], [python], or enabled [quarto] is required"));

        let quarto: Config = toml::from_str(
            r#"[project]
name = "quarto-only"

[quarto]
enabled = true
"#,
        )
        .unwrap();
        quarto.validate().unwrap();
    }

    #[test]
    fn starter_config_selects_languages_and_validates_versions() {
        let python = starter_config("python-only", Some("3.13"), None).unwrap();
        assert!(python.contains("[python]"));
        assert!(python.contains("version = \"3.13\""));
        assert!(!python.contains("[r]"));

        let r = starter_config("r-only", None, Some("4.4")).unwrap();
        assert!(r.contains("[r]"));
        assert!(!r.contains("[python]"));

        let mixed = starter_config("mixed", None, None).unwrap();
        assert!(mixed.contains("[r]"));
        assert!(mixed.contains("[python]"));
        assert_eq!(
            starter_config("my-project", None, None).unwrap(),
            DEFAULT_CONFIG
        );

        assert!(starter_config("invalid", Some("3.12;bad"), None).is_err());
    }

    #[test]
    fn enable_language_adds_defaults_and_updates_versions_without_losing_packages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dual.toml");
        fs::write(
            &path,
            starter_config("python-only", Some("3.13"), None).unwrap(),
        )
        .unwrap();

        let enabled = Config::enable_language(&path, "r", None).unwrap();
        assert!(enabled.changed);
        assert_eq!(enabled.previous_version, None);
        assert_eq!(enabled.version, DEFAULT_R_VERSION);
        let config = Config::from_path(&path).unwrap();
        assert!(config.r.enabled);
        assert!(config.python.enabled);

        Config::add_packages(&path, "r", &["dplyr".into()]).unwrap();
        let updated = Config::enable_language(&path, "r", Some("4.4")).unwrap();
        assert!(updated.changed);
        assert_eq!(updated.previous_version.as_deref(), Some(DEFAULT_R_VERSION));
        assert_eq!(updated.version, "4.4");
        let config = Config::from_path(&path).unwrap();
        assert_eq!(config.r.version, "4.4");
        assert_eq!(config.r.packages, ["dplyr"]);

        let unchanged = Config::enable_language(&path, "r", None).unwrap();
        assert!(!unchanged.changed);
        assert_eq!(unchanged.version, "4.4");
    }

    #[test]
    fn invalid_enable_version_does_not_modify_config() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dual.toml");
        fs::write(
            &path,
            starter_config("python-only", Some("3.12"), None).unwrap(),
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(Config::enable_language(&path, "r", Some("4.5;bad")).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), before);
    }

    #[test]
    fn disable_language_is_safe_and_force_removes_dependencies() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dual.toml");
        fs::write(&path, DEFAULT_CONFIG).unwrap();
        Config::add_packages(&path, "python", &["pandas".into()]).unwrap();

        let before = fs::read_to_string(&path).unwrap();
        let error = Config::disable_language(&path, "python", false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--force"));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        let disabled = Config::disable_language(&path, "python", true).unwrap();
        assert!(disabled.changed);
        assert_eq!(disabled.removed_packages, 1);
        assert_eq!(disabled.removed_indexes, 0);
        let config = Config::from_path(&path).unwrap();
        assert!(!config.python.enabled);
        assert!(config.r.enabled);

        let unchanged = Config::disable_language(&path, "python", false).unwrap();
        assert!(!unchanged.changed);
    }

    #[test]
    fn disable_language_blocks_referenced_tasks_even_with_force() {
        for (section, command) in [
            ("r", "Rscript analysis.R"),
            ("python", "python3.13 scripts/model.py"),
            ("r", "quarto render report.qmd"),
            ("python", "quarto render report.qmd"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("dual.toml");
            fs::write(
                &path,
                DEFAULT_CONFIG.replace("[tasks]\n", &format!("[tasks]\nanalysis = {command:?}\n")),
            )
            .unwrap();
            let before = fs::read_to_string(&path).unwrap();

            let error = Config::disable_language(&path, section, true)
                .unwrap_err()
                .to_string();
            assert!(error.contains("analysis"));
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
        }
    }

    #[test]
    fn disable_language_protects_the_last_runtime_except_for_quarto_projects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dual.toml");
        fs::write(
            &path,
            starter_config("python-only", Some("3.12"), None).unwrap(),
        )
        .unwrap();
        let error = Config::disable_language(&path, "python", true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("last project runtime"));

        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("enabled = false", "enabled = true"),
        )
        .unwrap();
        assert!(
            Config::disable_language(&path, "python", false)
                .unwrap()
                .changed
        );
        let config = Config::from_path(&path).unwrap();
        assert!(!config.python.enabled);
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
            .insert("unsafe".into(), TaskConfig::simple("echo\u{7}danger"));
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

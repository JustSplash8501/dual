use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::config::{self, Config};
use crate::security::{self, MAX_CONFIG_BYTES};

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ImportReport {
    pub source: String,
    pub python: Vec<String>,
    pub r: Vec<String>,
    pub python_version: Option<String>,
    pub r_version: Option<String>,
    pub skipped: Vec<String>,
}

impl ImportReport {
    fn is_empty(&self) -> bool {
        self.python.is_empty()
            && self.r.is_empty()
            && self.python_version.is_none()
            && self.r_version.is_none()
    }
}

#[derive(Clone, Debug, Default)]
struct ImportData {
    python: Vec<String>,
    r: Vec<String>,
    python_version: Option<String>,
    r_version: Option<String>,
    skipped: Vec<String>,
}

pub fn import_file(project_root: &Path, source: &Path) -> Result<ImportReport> {
    let path = if source.is_absolute() {
        source.to_owned()
    } else {
        project_root.join(source)
    };
    if !path.is_file() {
        anyhow::bail!("import source was not found: {}", path.display());
    }
    let contents = security::read_text_file(&path, MAX_CONFIG_BYTES, "import source")?;
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut data = if file_name == "requirements.txt" {
        parse_requirements(&contents)
    } else if file_name == "renv.lock" {
        parse_renv_lock(&contents)?
    } else if file_name == "uv.lock" {
        parse_uv_lock(&contents)?
    } else if file_name == "environment.yml" || file_name == "environment.yaml" {
        parse_environment_yml(&contents)?
    } else if file_name == "env.lock" {
        parse_env_lock(&contents)?
    } else {
        match extension.as_str() {
            "txt" => parse_requirements(&contents),
            "yml" | "yaml" => parse_environment_yml(&contents)?,
            "lock" => parse_env_lock(&contents)?,
            _ => anyhow::bail!(
                "unsupported import source. Expected requirements.txt, renv.lock, env.lock, uv.lock, or environment.yml"
            ),
        }
    };
    deduplicate(&mut data.python);
    deduplicate(&mut data.r);
    data.python
        .retain(|package| config::parse_python_requirement(package).is_ok());
    data.r
        .retain(|package| config::valid_r_package_reference(package));

    let report = ImportReport {
        source: path.display().to_string(),
        python: data.python.clone(),
        r: data.r.clone(),
        python_version: data.python_version.clone(),
        r_version: data.r_version.clone(),
        skipped: data.skipped.clone(),
    };
    if report.is_empty() {
        anyhow::bail!("no supported dependencies were found in {}", path.display());
    }

    apply_import(project_root, &data)?;
    Ok(report)
}

fn apply_import(project_root: &Path, data: &ImportData) -> Result<()> {
    let config_path = Config::path(project_root);
    let mut document = security::read_text_file(&config_path, MAX_CONFIG_BYTES, "dual.toml")?
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| crate::errors::DualError::InvalidConfig(error.to_string()))?;

    if let Some(version) = &data.python_version {
        set_string(&mut document, "python", "version", version)?;
    }
    if let Some(version) = &data.r_version {
        set_string(&mut document, "r", "version", version)?;
    }
    append_packages(&mut document, "python", "dependencies", &data.python)?;
    for package in &data.r {
        let (key, value) = r_key_value(package);
        append_packages(&mut document, "r", key, &[value.to_owned()])?;
    }

    security::write_file_atomic(&config_path, document.to_string().as_bytes(), "dual.toml")?;
    Config::from_path(&config_path)?;
    Ok(())
}

fn set_string(
    document: &mut toml_edit::DocumentMut,
    section: &str,
    key: &str,
    value: &str,
) -> Result<()> {
    let table = document
        .get_mut(section)
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| {
            crate::errors::DualError::InvalidConfig(format!("[{section}] is required"))
        })?;
    table[key] = toml_edit::value(value);
    Ok(())
}

fn append_packages(
    document: &mut toml_edit::DocumentMut,
    section: &str,
    key: &str,
    packages: &[String],
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let table = document
        .get_mut(section)
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| {
            crate::errors::DualError::InvalidConfig(format!("[{section}] is required"))
        })?;
    if !table.contains_key(key) {
        table.insert(
            key,
            toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new())),
        );
    }
    let array = table
        .get_mut(key)
        .and_then(toml_edit::Item::as_array_mut)
        .ok_or_else(|| {
            crate::errors::DualError::InvalidConfig(format!("array expected for `{key}`"))
        })?;
    let mut existing = array
        .iter()
        .filter_map(toml_edit::Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for package in packages {
        if !existing.iter().any(|value| value == package) {
            existing.push(package.clone());
        }
    }
    let mut replacement = toml_edit::Array::new();
    for value in existing {
        replacement.push(value);
    }
    *array = replacement;
    Ok(())
}

fn r_key_value(package: &str) -> (&str, &str) {
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

fn parse_requirements(contents: &str) -> ImportData {
    let mut data = ImportData::default();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('-') {
            data.skipped.push(line.to_owned());
            continue;
        }
        let requirement = strip_inline_comment(line).trim();
        if requirement.contains("://") || requirement.contains(" @ ") {
            data.skipped.push(requirement.to_owned());
        } else {
            data.python.push(requirement.to_owned());
        }
    }
    data
}

fn parse_renv_lock(contents: &str) -> Result<ImportData> {
    let lock: JsonValue = serde_json::from_str(contents).context("renv.lock is not valid JSON")?;
    let mut data = ImportData {
        r_version: lock
            .pointer("/R/Version")
            .and_then(JsonValue::as_str)
            .map(str::to_owned),
        ..ImportData::default()
    };
    let Some(packages) = lock.get("Packages").and_then(JsonValue::as_object) else {
        return Ok(data);
    };
    for (name, package) in packages {
        let source = package
            .get("Source")
            .and_then(JsonValue::as_str)
            .unwrap_or("CRAN");
        let version = package.get("Version").and_then(JsonValue::as_str);
        let package_name = package
            .get("Package")
            .and_then(JsonValue::as_str)
            .unwrap_or(name);
        match source {
            "CRAN" | "Repository" | "RSPM" => {
                data.r
                    .push(with_version(&format!("cran::{package_name}"), version));
            }
            "Bioconductor" => data
                .r
                .push(with_version(&format!("bioc::{package_name}"), version)),
            "GitHub" => {
                let owner = package.get("RemoteUsername").and_then(JsonValue::as_str);
                let repo = package.get("RemoteRepo").and_then(JsonValue::as_str);
                if let (Some(owner), Some(repo)) = (owner, repo) {
                    let reference = package
                        .get("RemoteSha")
                        .or_else(|| package.get("RemoteRef"))
                        .and_then(JsonValue::as_str);
                    let mut value = format!("github::{owner}/{repo}");
                    if let Some(reference) = reference {
                        value.push('@');
                        value.push_str(reference);
                    }
                    if repo != package_name {
                        value = format!("{package_name}={value}");
                    }
                    data.r.push(value);
                } else {
                    data.skipped.push(package_name.to_owned());
                }
            }
            _ => data.skipped.push(package_name.to_owned()),
        }
    }
    Ok(data)
}

fn parse_uv_lock(contents: &str) -> Result<ImportData> {
    #[derive(Deserialize)]
    struct UvLock {
        #[serde(default)]
        package: Vec<UvPackage>,
        #[serde(default, rename = "requires-python")]
        requires_python: Option<String>,
    }
    #[derive(Deserialize)]
    struct UvPackage {
        name: String,
        version: String,
    }

    let lock: UvLock = toml::from_str(contents).context("uv.lock is not valid TOML")?;
    let mut data = ImportData {
        python_version: lock.requires_python,
        ..ImportData::default()
    };
    for package in lock.package {
        data.python
            .push(format!("{}=={}", package.name, package.version));
    }
    Ok(data)
}

fn parse_environment_yml(contents: &str) -> Result<ImportData> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(contents).context("environment.yml is not valid YAML")?;
    let mut data = ImportData::default();
    let dependencies = value
        .get("dependencies")
        .and_then(serde_yaml::Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    for dependency in dependencies {
        match dependency {
            serde_yaml::Value::String(value) => parse_conda_dependency(&mut data, &value),
            serde_yaml::Value::Mapping(mapping) => {
                if let Some(pip) = mapping
                    .get(serde_yaml::Value::String("pip".to_owned()))
                    .and_then(serde_yaml::Value::as_sequence)
                {
                    for package in pip {
                        if let Some(package) = package.as_str() {
                            data.python.push(package.to_owned());
                        }
                    }
                }
            }
            _ => data.skipped.push(format!("{dependency:?}")),
        }
    }
    Ok(data)
}

fn parse_env_lock(contents: &str) -> Result<ImportData> {
    if let Ok(value) = serde_json::from_str::<JsonValue>(contents) {
        return Ok(parse_generic_json_lock(&value));
    }
    if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(contents) {
        return Ok(parse_generic_yaml_lock(&value));
    }
    parse_uv_lock(contents)
}

fn parse_generic_json_lock(value: &JsonValue) -> ImportData {
    let mut data = ImportData::default();
    collect_json_packages(value, &mut data);
    data
}

fn collect_json_packages(value: &JsonValue, data: &mut ImportData) {
    match value {
        JsonValue::Array(values) => values
            .iter()
            .for_each(|value| collect_json_packages(value, data)),
        JsonValue::Object(object) => {
            if let Some(name) = object.get("name").and_then(JsonValue::as_str) {
                let version = object.get("version").and_then(JsonValue::as_str);
                parse_locked_name(data, name, version);
            }
            for value in object.values() {
                collect_json_packages(value, data);
            }
        }
        _ => {}
    }
}

fn parse_generic_yaml_lock(value: &serde_yaml::Value) -> ImportData {
    let mut data = ImportData::default();
    collect_yaml_packages(value, &mut data);
    data
}

fn collect_yaml_packages(value: &serde_yaml::Value, data: &mut ImportData) {
    match value {
        serde_yaml::Value::Sequence(values) => values
            .iter()
            .for_each(|value| collect_yaml_packages(value, data)),
        serde_yaml::Value::Mapping(mapping) => {
            if let Some(name) = mapping
                .get(serde_yaml::Value::String("name".to_owned()))
                .and_then(serde_yaml::Value::as_str)
            {
                let version = mapping
                    .get(serde_yaml::Value::String("version".to_owned()))
                    .and_then(serde_yaml::Value::as_str);
                parse_locked_name(data, name, version);
            }
            for value in mapping.values() {
                collect_yaml_packages(value, data);
            }
        }
        _ => {}
    }
}

fn parse_conda_dependency(data: &mut ImportData, dependency: &str) {
    let name = dependency
        .split(['=', '<', '>', ' '])
        .next()
        .unwrap_or_default()
        .trim();
    let version = dependency
        .split_once('=')
        .map(|(_, rest)| rest.split('=').next().unwrap_or(rest).trim())
        .filter(|value| !value.is_empty() && value.chars().next().is_some_and(char::is_numeric));
    match name {
        "python" => data.python_version = version.map(str::to_owned),
        "r-base" => data.r_version = version.map(str::to_owned),
        _ if name.starts_with("r-") => {
            let package = name.trim_start_matches("r-");
            if !matches!(package, "base" | "essentials") {
                data.r
                    .push(with_version(&format!("cran::{package}"), version));
            }
        }
        _ => data.skipped.push(dependency.to_owned()),
    }
}

fn parse_locked_name(data: &mut ImportData, name: &str, version: Option<&str>) {
    if name == "python" {
        data.python_version = version.map(str::to_owned);
    } else if name == "r-base" {
        data.r_version = version.map(str::to_owned);
    } else if let Some(package) = name.strip_prefix("r-") {
        if !matches!(package, "base" | "essentials") {
            data.r
                .push(with_version(&format!("cran::{package}"), version));
        }
    } else if config::valid_distribution_name_for_import(name) {
        data.python.push(with_python_version(name, version));
    }
}

fn with_version(package: &str, version: Option<&str>) -> String {
    if let Some(version) = version {
        format!("{package}@{version}")
    } else {
        package.to_owned()
    }
}

fn with_python_version(name: &str, version: Option<&str>) -> String {
    if let Some(version) = version {
        format!("{name}=={version}")
    } else {
        name.to_owned()
    }
}

fn strip_inline_comment(line: &str) -> &str {
    line.split(" #").next().unwrap_or(line)
}

fn deduplicate(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

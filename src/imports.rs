use std::collections::{BTreeMap, BTreeSet};
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub python_indexes: Vec<String>,
    pub r: Vec<String>,
    pub python_version: Option<String>,
    pub r_version: Option<String>,
    pub skipped: Vec<String>,
}

impl ImportReport {
    fn is_empty(&self) -> bool {
        self.python.is_empty()
            && self.python_indexes.is_empty()
            && self.r.is_empty()
            && self.python_version.is_none()
            && self.r_version.is_none()
    }
}

#[derive(Clone, Debug, Default)]
struct ImportData {
    python: Vec<String>,
    python_indexes: Vec<String>,
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
    let canonical_project_root = project_root.canonicalize().with_context(|| {
        format!(
            "could not canonicalize project root {}",
            project_root.display()
        )
    })?;
    let canonical_source = path
        .canonicalize()
        .with_context(|| format!("could not canonicalize import source {}", path.display()))?;
    if !canonical_source.starts_with(&canonical_project_root) {
        anyhow::bail!(
            "import source must be inside the project: {}",
            path.display()
        );
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
    } else if file_name == "pyproject.toml" {
        parse_pyproject_toml(&contents)?
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
            "toml" => parse_pyproject_toml(&contents)?,
            "yml" | "yaml" => parse_environment_yml(&contents)?,
            "lock" => parse_env_lock(&contents)?,
            _ => anyhow::bail!(
                "unsupported import source. Expected pyproject.toml, requirements.txt, renv.lock, env.lock, uv.lock, or environment.yml"
            ),
        }
    };
    deduplicate(&mut data.python);
    deduplicate(&mut data.python_indexes);
    deduplicate(&mut data.r);
    let mut skipped = Vec::new();
    data.python.retain(|package| {
        if config::parse_python_requirement(package).is_ok() {
            true
        } else {
            skipped.push(package.clone());
            false
        }
    });
    data.r.retain(|package| {
        if config::valid_r_package_reference(package) {
            true
        } else {
            skipped.push(package.clone());
            false
        }
    });
    data.python_indexes.retain(|index| {
        if config::validate_index_url(index).is_ok() {
            true
        } else {
            skipped.push(index.clone());
            false
        }
    });
    data.skipped.extend(skipped);

    let report = ImportReport {
        source: path.display().to_string(),
        python: data.python.clone(),
        python_indexes: data.python_indexes.clone(),
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
    append_python_indexes(&mut document, &data.python_indexes)?;
    let mut r_packages = BTreeMap::<&str, Vec<&str>>::new();
    for package in &data.r {
        let (key, value) = r_key_value(package);
        r_packages.entry(key).or_default().push(value);
    }
    for (key, packages) in r_packages {
        append_package_values(&mut document, "r", key, packages.into_iter())?;
    }

    let rendered = document.to_string();
    let config = toml::from_str::<Config>(&rendered)
        .map_err(|error| crate::errors::DualError::InvalidConfig(error.to_string()))?;
    config.validate()?;
    security::write_file_atomic(&config_path, rendered.as_bytes(), "dual.toml")?;
    Ok(())
}

fn append_python_indexes(document: &mut toml_edit::DocumentMut, indexes: &[String]) -> Result<()> {
    if indexes.is_empty() {
        return Ok(());
    }
    config::ensure_language_table(document, "python")?;
    let table = document
        .get_mut("python")
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| crate::errors::DualError::InvalidConfig("[python] is required".into()))?;
    if !table.contains_key("index") {
        table.insert(
            "index",
            toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }
    let entries = table
        .get_mut("index")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            crate::errors::DualError::InvalidConfig(
                "array of tables expected for `python.index`".into(),
            )
        })?;
    let mut seen = entries
        .iter()
        .filter_map(|entry| entry.get("url").and_then(toml_edit::Item::as_str))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    for index in indexes {
        if seen.insert(index.clone()) {
            let mut entry = toml_edit::Table::new();
            entry.insert("url", toml_edit::value(index));
            entries.push(entry);
        }
    }
    Ok(())
}

fn set_string(
    document: &mut toml_edit::DocumentMut,
    section: &str,
    key: &str,
    value: &str,
) -> Result<()> {
    config::ensure_language_table(document, section)?;
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
    append_package_values(document, section, key, packages.iter().map(String::as_str))
}

fn append_package_values<'a>(
    document: &mut toml_edit::DocumentMut,
    section: &str,
    key: &str,
    packages: impl Iterator<Item = &'a str>,
) -> Result<()> {
    config::ensure_language_table(document, section)?;
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
    let mut seen = existing.iter().cloned().collect::<BTreeSet<_>>();
    for package in packages {
        let package = package.to_owned();
        if seen.insert(package.clone()) {
            existing.push(package);
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
    let mut primary_index = None;
    let mut extra_indexes = Vec::new();
    for raw in requirement_logical_lines(contents) {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let uncommented = strip_inline_comment(line).trim();
        if let Some((primary, index)) = requirement_index_url(uncommented) {
            if primary {
                primary_index = Some(index.to_owned());
            } else {
                extra_indexes.push(index.to_owned());
            }
            continue;
        }
        if line.starts_with('-') {
            data.skipped.push(line.to_owned());
            continue;
        }
        let normalized = strip_requirement_hashes(uncommented);
        let requirement = normalized.trim();
        if requirement.is_empty() {
            continue;
        }
        if requirement.contains("://") || requirement.contains(" @ ") {
            data.skipped.push(requirement.to_owned());
        } else {
            data.python.push(requirement.to_owned());
        }
    }
    if let Some(primary) = primary_index {
        data.python_indexes.push(primary);
    } else if !extra_indexes.is_empty() {
        data.python_indexes
            .push("https://pypi.org/simple".to_owned());
    }
    data.python_indexes.extend(extra_indexes);
    data
}

fn requirement_index_url(line: &str) -> Option<(bool, &str)> {
    for (option, primary) in [
        ("--index-url", true),
        ("--extra-index-url", false),
        ("-i", true),
    ] {
        if let Some(value) = line.strip_prefix(option) {
            if !value.is_empty() && !value.starts_with(['=', ' ', '\t']) {
                continue;
            }
            let value = value.strip_prefix('=').unwrap_or(value).trim_start();
            if !value.is_empty() {
                return Some((primary, value));
            }
        }
    }
    None
}

fn requirement_logical_lines(contents: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for raw in contents.lines() {
        let trimmed = raw.trim();
        let continued = trimmed.ends_with('\\');
        let segment = trimmed.strip_suffix('\\').unwrap_or(trimmed).trim_end();
        if !current.is_empty() && !segment.is_empty() {
            current.push(' ');
        }
        current.push_str(segment);
        if !continued {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn strip_requirement_hashes(value: &str) -> String {
    let mut retained = Vec::new();
    let mut skip_next = false;
    for part in value.split_whitespace() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if part == "--hash" {
            skip_next = true;
        } else if !part.starts_with("--hash=") {
            retained.push(part);
        }
    }
    retained.join(" ")
}

fn parse_pyproject_toml(contents: &str) -> Result<ImportData> {
    let value: toml::Value =
        toml::from_str(contents).context("pyproject.toml is not valid TOML")?;
    let mut data = ImportData::default();

    if let Some(project) = value.get("project").and_then(toml::Value::as_table) {
        data.python_version = project
            .get("requires-python")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        collect_string_array(
            &mut data,
            project.get("dependencies").and_then(toml::Value::as_array),
        );
        if let Some(optional) = project
            .get("optional-dependencies")
            .and_then(toml::Value::as_table)
        {
            for dependencies in optional.values().filter_map(toml::Value::as_array) {
                collect_string_array(&mut data, Some(dependencies));
            }
        }
    }

    if let Some(poetry) = value
        .get("tool")
        .and_then(toml::Value::as_table)
        .and_then(|tool| tool.get("poetry"))
        .and_then(toml::Value::as_table)
    {
        collect_poetry_dependency_table(
            &mut data,
            poetry.get("dependencies").and_then(toml::Value::as_table),
        );
        if let Some(groups) = poetry.get("group").and_then(toml::Value::as_table) {
            for group in groups.values().filter_map(toml::Value::as_table) {
                collect_poetry_dependency_table(
                    &mut data,
                    group.get("dependencies").and_then(toml::Value::as_table),
                );
            }
        }
        if let Some(dev) = poetry
            .get("dev-dependencies")
            .and_then(toml::Value::as_table)
        {
            collect_poetry_dependency_table(&mut data, Some(dev));
        }
        if let Some(sources) = poetry.get("source").and_then(toml::Value::as_array) {
            collect_index_tables(&mut data, sources);
        }
    }

    if let Some(groups) = value
        .get("dependency-groups")
        .and_then(toml::Value::as_table)
    {
        for dependencies in groups.values().filter_map(toml::Value::as_array) {
            collect_string_array(&mut data, Some(dependencies));
        }
    }

    if let Some(indexes) = value
        .get("tool")
        .and_then(toml::Value::as_table)
        .and_then(|tool| tool.get("uv"))
        .and_then(toml::Value::as_table)
        .and_then(|uv| uv.get("index"))
        .and_then(toml::Value::as_array)
    {
        collect_index_tables(&mut data, indexes);
    }

    Ok(data)
}

fn collect_index_tables(data: &mut ImportData, indexes: &[toml::Value]) {
    for index in indexes {
        if let Some(url) = index
            .as_table()
            .and_then(|index| index.get("url"))
            .and_then(toml::Value::as_str)
        {
            data.python_indexes.push(url.to_owned());
        } else {
            data.skipped.push(format!("{index:?}"));
        }
    }
}

fn collect_string_array(data: &mut ImportData, values: Option<&Vec<toml::Value>>) {
    let Some(values) = values else {
        return;
    };
    for value in values {
        if let Some(requirement) = value.as_str() {
            data.python.push(requirement.to_owned());
        } else {
            data.skipped.push(format!("{value:?}"));
        }
    }
}

fn collect_poetry_dependency_table(
    data: &mut ImportData,
    dependencies: Option<&toml::map::Map<String, toml::Value>>,
) {
    let Some(dependencies) = dependencies else {
        return;
    };
    for (name, dependency) in dependencies {
        if name.eq_ignore_ascii_case("python") {
            if data.python_version.is_none() {
                data.python_version = dependency.as_str().map(str::to_owned);
            }
            continue;
        }
        match poetry_dependency_to_requirement(name, dependency) {
            Some(requirement) => data.python.push(requirement),
            None => data.skipped.push(format!("{name} = {dependency:?}")),
        }
    }
}

fn poetry_dependency_to_requirement(name: &str, dependency: &toml::Value) -> Option<String> {
    if let Some(version) = dependency.as_str() {
        return format_poetry_requirement(name, version, &[]);
    }
    let table = dependency.as_table()?;
    if table.contains_key("git")
        || table.contains_key("url")
        || table.contains_key("path")
        || table.contains_key("file")
        || table.get("optional").and_then(toml::Value::as_bool) == Some(true)
    {
        return None;
    }
    let version = table
        .get("version")
        .and_then(toml::Value::as_str)
        .unwrap_or("*");
    let extras = table
        .get("extras")
        .and_then(toml::Value::as_array)
        .map(|extras| {
            extras
                .iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if table.get("extras").is_some() && extras.is_empty() {
        return None;
    }
    format_poetry_requirement(name, version, &extras)
}

fn format_poetry_requirement(name: &str, version: &str, extras: &[&str]) -> Option<String> {
    let extras = if extras.is_empty() {
        String::new()
    } else {
        format!("[{}]", extras.join(","))
    };
    if version.trim() == "*" {
        Some(format!("{name}{extras}"))
    } else if poetry_version_is_pep508(version) {
        Some(format!("{name}{extras}{version}"))
    } else {
        None
    }
}

fn poetry_version_is_pep508(version: &str) -> bool {
    let value = version.trim_start();
    [">=", "<=", "==", "!=", "~=", ">", "<", "==="]
        .iter()
        .any(|operator| value.starts_with(operator))
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
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        source: Option<toml::Value>,
    }

    let lock: UvLock = toml::from_str(contents).context("uv.lock is not valid TOML")?;
    let mut data = ImportData {
        python_version: lock.requires_python,
        ..ImportData::default()
    };
    for package in lock.package {
        let local = package
            .source
            .as_ref()
            .and_then(toml::Value::as_table)
            .is_some_and(|source| {
                ["editable", "virtual", "directory", "path"]
                    .iter()
                    .any(|key| source.contains_key(*key))
            });
        match (local, package.version) {
            (false, Some(version)) => data.python.push(format!("{}=={version}", package.name)),
            _ => data
                .skipped
                .push(format!("{} (local uv package)", package.name)),
        }
    }
    Ok(data)
}

fn parse_environment_yml(contents: &str) -> Result<ImportData> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(contents).context("environment.yml is not valid YAML")?;
    let mut data = ImportData::default();
    let Some(dependencies) = value
        .get("dependencies")
        .and_then(serde_yaml::Value::as_sequence)
    else {
        return Ok(data);
    };
    for dependency in dependencies {
        match dependency {
            serde_yaml::Value::String(value) => parse_conda_dependency(&mut data, value),
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
    let unqualified = dependency
        .rsplit_once("::")
        .map(|(_, dependency)| dependency)
        .unwrap_or(dependency);
    let name = unqualified
        .split(['=', '<', '>', ' '])
        .next()
        .unwrap_or_default()
        .trim();
    let version = unqualified
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

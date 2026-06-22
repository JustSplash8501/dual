use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::{
    parse_python_requirement, valid_r_package_reference, valid_version_specifier,
    validate_index_url, PackageIndex,
};
use crate::security;

const MAX_SCRIPT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptKind {
    Python,
    R,
    Quarto,
    RMarkdown,
}

impl ScriptKind {
    pub fn from_path(path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "py" => Ok(Self::Python),
            "r" => Ok(Self::R),
            "qmd" => Ok(Self::Quarto),
            "rmd" => Ok(Self::RMarkdown),
            _ => anyhow::bail!(
                "unsupported script type for {}. Expected .py, .R, .qmd, or .Rmd",
                path.display()
            ),
        }
    }

    fn uses_html_comments(self) -> bool {
        matches!(self, Self::Quarto | Self::RMarkdown)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptMetadata {
    pub python_version: Option<String>,
    pub r_version: Option<String>,
    pub python_dependencies: Vec<String>,
    pub python_indexes: Vec<PackageIndex>,
    pub cran: Vec<String>,
    pub bioc: Vec<String>,
    pub github: Vec<String>,
}

#[derive(Debug)]
pub struct ParsedMetadata {
    pub metadata: ScriptMetadata,
    range: std::ops::Range<usize>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetadata {
    #[serde(rename = "requires-python")]
    requires_python: Option<String>,
    python: Option<String>,
    r: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default, rename = "python-dependencies")]
    python_dependencies: Vec<String>,
    #[serde(default)]
    cran: Vec<String>,
    #[serde(default)]
    bioc: Vec<String>,
    #[serde(default)]
    github: Vec<String>,
    #[serde(default)]
    tool: ToolMetadata,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolMetadata {
    #[serde(default)]
    dual: DualToolMetadata,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DualToolMetadata {
    #[serde(default)]
    index: Vec<PackageIndex>,
}

pub fn read(path: &Path) -> Result<Option<ParsedMetadata>> {
    let contents = security::read_text_file(path, MAX_SCRIPT_BYTES, "script")?;
    parse(&contents, ScriptKind::from_path(path)?)
        .with_context(|| format!("invalid inline metadata in {}", path.display()))
}

pub fn parse(contents: &str, kind: ScriptKind) -> Result<Option<ParsedMetadata>> {
    let Some((range, body)) = extract_block(contents, kind)? else {
        return Ok(None);
    };
    let raw: RawMetadata = toml::from_str(&body)
        .map_err(|error| anyhow::anyhow!("metadata TOML is invalid: {error}"))?;
    let metadata = normalize(raw, kind)?;
    Ok(Some(ParsedMetadata { metadata, range }))
}

fn normalize(raw: RawMetadata, kind: ScriptKind) -> Result<ScriptMetadata> {
    if raw.requires_python.is_some() && raw.python.is_some() {
        anyhow::bail!("use only one of `requires-python` or `python`");
    }
    let python_version = raw.requires_python.or(raw.python);
    let mut python_dependencies = raw.dependencies;
    python_dependencies.extend(raw.python_dependencies);
    deduplicate(&mut python_dependencies);

    let mut metadata = ScriptMetadata {
        python_version,
        r_version: raw.r,
        python_dependencies,
        python_indexes: raw.tool.dual.index,
        cran: raw.cran,
        bioc: raw.bioc,
        github: raw.github,
    };
    deduplicate(&mut metadata.cran);
    deduplicate(&mut metadata.bioc);
    deduplicate(&mut metadata.github);
    deduplicate_indexes(&mut metadata.python_indexes);

    for index in &metadata.python_indexes {
        validate_index_url(&index.url)?;
    }
    validate_values(&metadata)?;

    match kind {
        ScriptKind::Python
            if metadata.r_version.is_some()
                || !metadata.cran.is_empty()
                || !metadata.bioc.is_empty()
                || !metadata.github.is_empty() =>
        {
            anyhow::bail!("Python script metadata cannot contain R dependency fields")
        }
        ScriptKind::R
            if metadata.python_version.is_some()
                || !metadata.python_dependencies.is_empty()
                || !metadata.python_indexes.is_empty() =>
        {
            anyhow::bail!("R script metadata cannot contain Python dependency fields")
        }
        _ => {}
    }
    Ok(metadata)
}

fn validate_values(metadata: &ScriptMetadata) -> Result<()> {
    for (field, value) in [
        ("Python version", metadata.python_version.as_deref()),
        ("R version", metadata.r_version.as_deref()),
    ] {
        if let Some(value) = value {
            if value.trim().is_empty()
                || security::contains_control_characters(value)
                || !valid_version_specifier(value)
            {
                anyhow::bail!("{field} is not a supported version requirement");
            }
        }
    }
    for (field, values) in [
        ("Python dependency", &metadata.python_dependencies),
        ("CRAN package", &metadata.cran),
        ("Bioconductor package", &metadata.bioc),
        ("GitHub package", &metadata.github),
    ] {
        for value in values {
            if value.trim().is_empty() || security::contains_control_characters(value) {
                anyhow::bail!("{field} names cannot be empty or contain control characters");
            }
        }
    }
    for dependency in &metadata.python_dependencies {
        parse_python_requirement(dependency)
            .with_context(|| format!("invalid Python dependency {dependency:?}"))?;
    }
    for package in &metadata.cran {
        if !valid_r_package_reference(&format!("cran::{package}")) {
            anyhow::bail!("invalid CRAN package {package:?}");
        }
    }
    for package in &metadata.bioc {
        if !valid_r_package_reference(&format!("bioc::{package}")) {
            anyhow::bail!("invalid Bioconductor package {package:?}");
        }
    }
    for package in &metadata.github {
        if !valid_r_package_reference(&format!("github::{package}")) {
            anyhow::bail!("invalid GitHub package {package:?}; expected OWNER/REPO");
        }
    }
    Ok(())
}

fn extract_block(
    contents: &str,
    kind: ScriptKind,
) -> Result<Option<(std::ops::Range<usize>, String)>> {
    let (opening, closing) = if kind.uses_html_comments() {
        ("<!-- /// script", "/// -->")
    } else {
        ("# /// script", "# ///")
    };
    let Some(start) = find_line(contents, opening) else {
        return Ok(None);
    };
    let opening_end = contents[start..]
        .find('\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(contents.len());
    let Some(close_start) = find_line_from(contents, closing, opening_end) else {
        anyhow::bail!("metadata block starts with `{opening}` but has no `{closing}` terminator");
    };
    let end = contents[close_start..]
        .find('\n')
        .map(|offset| close_start + offset + 1)
        .unwrap_or(contents.len());
    let raw_body = &contents[opening_end..close_start];
    let body = if kind.uses_html_comments() {
        raw_body.to_owned()
    } else {
        let mut lines = Vec::new();
        for line in raw_body.lines() {
            let line = line.trim_end_matches('\r');
            let Some(line) = line.strip_prefix('#') else {
                anyhow::bail!("every line in a script metadata block must start with `#`");
            };
            lines.push(line.strip_prefix(' ').unwrap_or(line));
        }
        lines.join("\n")
    };
    Ok(Some((start..end, body)))
}

fn find_line(contents: &str, expected: &str) -> Option<usize> {
    find_line_from(contents, expected, 0)
}

fn find_line_from(contents: &str, expected: &str, from: usize) -> Option<usize> {
    let mut offset = from;
    for line in contents[from..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']).trim() == expected {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

pub fn initialize(path: &Path, python: Option<&str>, r: Option<&str>, force: bool) -> Result<()> {
    let kind = ScriptKind::from_path(path)?;
    match kind {
        ScriptKind::Python if r.is_some() => {
            anyhow::bail!("`--r` cannot be used when initializing a Python script")
        }
        ScriptKind::R if python.is_some() => {
            anyhow::bail!("`--python` cannot be used when initializing an R script")
        }
        _ => {}
    }
    let exists = path.exists();
    let contents = if exists {
        security::read_text_file(path, MAX_SCRIPT_BYTES, "script")?
    } else {
        String::new()
    };
    let existing = parse(&contents, kind)?;
    if existing.is_some() && !force {
        anyhow::bail!(
            "{} already contains inline metadata. Use `--force` to replace it.",
            path.display()
        );
    }

    let metadata = starter_metadata(kind, python, r);
    let block = render(&metadata, kind);
    let output = if let Some(existing) = existing {
        replace_range(&contents, existing.range, &block)
    } else if exists {
        insert_block(&contents, &block, kind)
    } else {
        format!("{block}\n{}", hello_world(kind))
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    security::write_file_atomic(path, output.as_bytes(), "script")
}

fn starter_metadata(kind: ScriptKind, python: Option<&str>, r: Option<&str>) -> ScriptMetadata {
    let mut metadata = ScriptMetadata::default();
    match kind {
        ScriptKind::Python => metadata.python_version = Some(python.unwrap_or("3.12").to_owned()),
        ScriptKind::R => metadata.r_version = Some(r.unwrap_or("4.5").to_owned()),
        ScriptKind::Quarto | ScriptKind::RMarkdown => {
            if python.is_none() && r.is_none() {
                metadata.python_version = Some("3.12".to_owned());
                metadata.r_version = Some("4.5".to_owned());
            } else {
                metadata.python_version = python.map(str::to_owned);
                metadata.r_version = r.map(str::to_owned);
            }
        }
    }
    metadata
}

fn insert_block(contents: &str, block: &str, kind: ScriptKind) -> String {
    if !kind.uses_html_comments() && contents.starts_with("#!") {
        if let Some(newline) = contents.find('\n') {
            return format!(
                "{}\n{block}\n{}",
                &contents[..newline],
                &contents[newline + 1..]
            );
        }
        return format!("{contents}\n{block}\n");
    }
    format!("{block}\n{contents}")
}

fn replace_range(contents: &str, range: std::ops::Range<usize>, block: &str) -> String {
    format!(
        "{}{}{}",
        &contents[..range.start],
        block,
        &contents[range.end..]
    )
}

fn hello_world(kind: ScriptKind) -> &'static str {
    match kind {
        ScriptKind::Python => "print(\"Hello from dual!\")\n",
        ScriptKind::R => "cat(\"Hello from dual!\\n\")\n",
        ScriptKind::Quarto => {
            "---\ntitle: \"Hello from dual\"\nformat: html\n---\n\n```{python}\nprint(\"Hello from dual!\")\n```\n"
        }
        ScriptKind::RMarkdown => {
            "---\ntitle: \"Hello from dual\"\noutput: html_document\n---\n\n```{r}\ncat(\"Hello from dual!\\n\")\n```\n"
        }
    }
}

pub struct AddOptions<'a> {
    pub language: Option<ScriptLanguage>,
    pub packages: &'a [String],
    pub index: Option<&'a str>,
    pub github: Option<&'a str>,
    pub bioc: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptLanguage {
    Python,
    R,
}

pub fn add(path: &Path, options: AddOptions<'_>) -> Result<()> {
    let kind = ScriptKind::from_path(path)?;
    let contents = security::read_text_file(path, MAX_SCRIPT_BYTES, "script")?;
    let parsed = parse(&contents, kind)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no inline metadata block. Run `dual init --script {}` first.",
            path.display(),
            path.display()
        )
    })?;
    let mut metadata = parsed.metadata;
    let language = resolve_language(kind, &options)?;

    if let Some(index) = options.index {
        if language != ScriptLanguage::Python {
            anyhow::bail!("`--index` can only be used with Python dependencies");
        }
        validate_index_url(index)?;
        metadata.python_indexes.push(PackageIndex {
            url: index.to_owned(),
        });
    }
    if let Some(repository) = options.github {
        if language != ScriptLanguage::R {
            anyhow::bail!("`--github` can only be used with R dependencies");
        }
        metadata.github.push(repository.to_owned());
    }
    match language {
        ScriptLanguage::Python => {
            if options.bioc || options.github.is_some() {
                anyhow::bail!("R package source flags cannot be used with Python dependencies");
            }
            metadata
                .python_dependencies
                .extend(options.packages.iter().cloned());
        }
        ScriptLanguage::R => {
            if options.index.is_some() {
                anyhow::bail!("`--index` cannot be used with R dependencies");
            }
            if options.bioc {
                metadata.bioc.extend(options.packages.iter().cloned());
            } else {
                metadata.cran.extend(options.packages.iter().cloned());
            }
        }
    }
    if options.packages.is_empty() && options.index.is_none() && options.github.is_none() {
        anyhow::bail!("provide at least one package or package source");
    }
    deduplicate(&mut metadata.python_dependencies);
    deduplicate(&mut metadata.cran);
    deduplicate(&mut metadata.bioc);
    deduplicate(&mut metadata.github);
    deduplicate_indexes(&mut metadata.python_indexes);
    validate_values(&metadata)?;
    let block = render(&metadata, kind);
    let output = replace_range(&contents, parsed.range, &block);
    security::write_file_atomic(path, output.as_bytes(), "script")
}

fn resolve_language(kind: ScriptKind, options: &AddOptions<'_>) -> Result<ScriptLanguage> {
    match kind {
        ScriptKind::Python => {
            if options.language == Some(ScriptLanguage::R) {
                anyhow::bail!("`--r` cannot be used with a Python script");
            }
            Ok(ScriptLanguage::Python)
        }
        ScriptKind::R => {
            if options.language == Some(ScriptLanguage::Python) {
                anyhow::bail!("`--python` cannot be used with an R script");
            }
            Ok(ScriptLanguage::R)
        }
        ScriptKind::Quarto | ScriptKind::RMarkdown => {
            if let Some(language) = options.language {
                return Ok(language);
            }
            if options.index.is_some() {
                return Ok(ScriptLanguage::Python);
            }
            if options.github.is_some() || options.bioc {
                return Ok(ScriptLanguage::R);
            }
            anyhow::bail!("use `--python` or `--r` when adding to a Quarto or R Markdown file")
        }
    }
}

pub fn render(metadata: &ScriptMetadata, kind: ScriptKind) -> String {
    let mut lines = Vec::new();
    match kind {
        ScriptKind::Python => {
            if let Some(version) = &metadata.python_version {
                lines.push(format!("requires-python = {}", toml_string(version)));
            }
            lines.extend(render_array("dependencies", &metadata.python_dependencies));
        }
        ScriptKind::R => {
            if let Some(version) = &metadata.r_version {
                lines.push(format!("r = {}", toml_string(version)));
            }
            lines.extend(render_array("cran", &metadata.cran));
            lines.extend(render_array("bioc", &metadata.bioc));
            lines.extend(render_array("github", &metadata.github));
        }
        ScriptKind::Quarto | ScriptKind::RMarkdown => {
            if let Some(version) = &metadata.python_version {
                lines.push(format!("python = {}", toml_string(version)));
            }
            if let Some(version) = &metadata.r_version {
                lines.push(format!("r = {}", toml_string(version)));
            }
            lines.extend(render_array(
                "python-dependencies",
                &metadata.python_dependencies,
            ));
            lines.extend(render_array("cran", &metadata.cran));
            lines.extend(render_array("bioc", &metadata.bioc));
            lines.extend(render_array("github", &metadata.github));
        }
    }
    for index in &metadata.python_indexes {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("[[tool.dual.index]]".to_owned());
        lines.push(format!("url = {}", toml_string(&index.url)));
    }

    if kind.uses_html_comments() {
        format!("<!-- /// script\n{}\n/// -->\n", lines.join("\n"))
    } else {
        let body = lines
            .iter()
            .map(|line| {
                if line.is_empty() {
                    "#".to_owned()
                } else {
                    format!("# {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("# /// script\n{body}\n# ///\n")
    }
}

fn render_array(name: &str, values: &[String]) -> Vec<String> {
    if values.is_empty() {
        return vec![format!("{name} = []")];
    }
    let mut lines = vec![format!("{name} = [")];
    lines.extend(
        values
            .iter()
            .map(|value| format!("  {},", toml_string(value))),
    );
    lines.push("]".to_owned());
    lines
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn deduplicate(values: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn deduplicate_indexes(indexes: &mut Vec<PackageIndex>) {
    let mut seen = std::collections::BTreeSet::new();
    indexes.retain(|index| seen.insert(index.url.clone()));
}

pub fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_pep_723_metadata() {
        let parsed = parse(
            "# /// script\n# requires-python = \">=3.12\"\n# dependencies = [\"rich\"]\n# [[tool.dual.index]]\n# url = \"https://example.com/simple\"\n# ///\n",
            ScriptKind::Python,
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.metadata.python_version.as_deref(), Some(">=3.12"));
        assert_eq!(parsed.metadata.python_dependencies, ["rich"]);
        assert_eq!(parsed.metadata.python_indexes.len(), 1);
    }

    #[test]
    fn parses_r_and_document_metadata() {
        let r = parse(
            "# /// script\n# r = \">=4.4\"\n# cran = [\"tidyverse\"]\n# bioc = [\"DESeq2\"]\n# github = [\"hadley/emo\"]\n# ///\n",
            ScriptKind::R,
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.metadata.bioc, ["DESeq2"]);

        let document = parse(
            "<!-- /// script\npython = \">=3.12\"\nr = \">=4.4\"\npython-dependencies = [\"pandas\"]\ncran = [\"knitr\"]\n/// -->\n",
            ScriptKind::Quarto,
        )
        .unwrap()
        .unwrap();
        assert_eq!(document.metadata.python_dependencies, ["pandas"]);
        assert_eq!(document.metadata.cran, ["knitr"]);
    }

    #[test]
    fn rejects_invalid_and_unterminated_metadata() {
        let invalid = parse(
            "# /// script\n# dependencies = [\n# ///\n",
            ScriptKind::Python,
        )
        .unwrap_err()
        .to_string();
        assert!(invalid.contains("metadata TOML is invalid"));

        let unterminated = parse("# /// script\n# dependencies = []\n", ScriptKind::Python)
            .unwrap_err()
            .to_string();
        assert!(unterminated.contains("has no"));

        let invalid_dependency = parse(
            "# /// script\n# dependencies = [\"requests; unsafe\"]\n# ///\n",
            ScriptKind::Python,
        )
        .unwrap_err()
        .to_string();
        assert!(invalid_dependency.contains("invalid Python dependency"));
    }
}

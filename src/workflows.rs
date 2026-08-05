use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::backend::{Backend, EnvironmentBackend};
use crate::config::{Config, EffectiveConfig, TaskConfig};
use crate::metadata::ScriptKind;
use crate::project_env::ProjectEnvironment;
use crate::security::{self, MAX_CONFIG_BYTES};

const SCRIPT_TASK: &str = "__dual_script";

pub fn looks_like_script(target: &str) -> bool {
    Path::new(target)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "py" | "r" | "qmd" | "rmd"
            )
        })
}

pub fn run_script(
    path: &Path,
    args: &[String],
    verbose: bool,
    trust_project: bool,
    no_install: bool,
    dry_run: bool,
) -> Result<()> {
    let mut effective = Config::for_script(path)?;
    prepare_script_config(&mut effective)?;
    let command = effective
        .config
        .tasks
        .get(SCRIPT_TASK)
        .expect("script task is configured");
    if dry_run {
        println!("Would prepare dependencies from {}.", effective.source);
        print_dependencies(
            &effective.config,
            Some(effective.source.to_string().as_str()),
        );
        println!("Would run: {}", command_with_args(command.command(), args));
        return Ok(());
    }

    let backend = EnvironmentBackend::for_script(&effective.root, &effective.script, verbose);
    let trust = security::ensure_project_trusted(&effective.root, trust_project)?;
    let project_environment = ProjectEnvironment::load(&effective.root)?;
    backend.ensure_available()?;
    security::verify_project_unchanged(&effective.root, &trust)?;
    if no_install {
        if !backend.environment_exists() {
            anyhow::bail!(
                "The script environment has not been prepared. Run `dual sync --script {}` first.",
                effective.script.display()
            );
        }
        backend.verify_manifest(&effective.config)?;
    } else {
        backend.init_or_update(&effective.config, true)?;
        security::verify_project_unchanged(&effective.root, &trust)?;
        security::refresh_project_trust(&effective.root)?;
    }
    security::verify_project_unchanged(&effective.root, &trust)?;
    println!("Running {}...", effective.script.display());
    let document_snapshot = matches!(effective.kind, ScriptKind::Quarto | ScriptKind::RMarkdown)
        .then(|| {
            security::snapshot_project_excluding(
                &effective.root,
                &document_outputs(&effective.root, &effective.script),
            )
        })
        .transpose()?;
    backend.run_with_environment(&effective.config, SCRIPT_TASK, args, &project_environment)?;
    if let Some(snapshot) = document_snapshot {
        security::verify_project_snapshot(&effective.root, &snapshot)
    } else {
        security::verify_project_unchanged(&effective.root, &trust)
    }
}

fn command_with_args(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_owned()
    } else {
        format!("{command} {}", args.join(" "))
    }
}

pub fn sync_script(path: &Path, verbose: bool, trust_project: bool, dry_run: bool) -> Result<()> {
    let mut effective = Config::for_script(path)?;
    prepare_script_config(&mut effective)?;
    if dry_run {
        println!("Would prepare dependencies from {}.", effective.source);
        print_dependencies(
            &effective.config,
            Some(effective.source.to_string().as_str()),
        );
        return Ok(());
    }
    let backend = EnvironmentBackend::for_script(&effective.root, &effective.script, verbose);
    let trust = security::ensure_project_trusted(&effective.root, trust_project)?;
    backend.ensure_available()?;
    security::verify_project_unchanged(&effective.root, &trust)?;
    backend.init_or_update(&effective.config, true)?;
    security::verify_project_unchanged(&effective.root, &trust)?;
    security::refresh_project_trust(&effective.root)?;
    println!("Script environment is ready.");
    Ok(())
}

pub fn sync_project(root: &Path, verbose: bool, trust_project: bool, dry_run: bool) -> Result<()> {
    let config = Config::load(root)?;
    if dry_run {
        println!("Would prepare dependencies from project dual.toml.");
        print_dependencies(&config, Some("project dual.toml"));
        return Ok(());
    }
    // Project sync has the same lock semantics as plain `dual up`: an
    // existing shared lock is enforced. Re-resolution stays an explicit
    // `dual up --refresh` operation.
    sync_config(root, &config, verbose, trust_project, false)?;
    println!("Project environment is ready.");
    Ok(())
}

fn sync_config(
    root: &Path,
    config: &Config,
    verbose: bool,
    trust_project: bool,
    refresh: bool,
) -> Result<()> {
    let backend = EnvironmentBackend::new(root, verbose);
    let trust = security::ensure_project_trusted(root, trust_project)?;
    backend.ensure_available()?;
    security::verify_project_unchanged(root, &trust)?;
    backend.init_or_update(config, refresh)?;
    security::verify_project_unchanged(root, &trust)?;
    security::refresh_project_trust(root)
}

fn prepare_script_config(effective: &mut EffectiveConfig) -> Result<()> {
    if effective.kind == ScriptKind::RMarkdown {
        effective.config.r.enabled = true;
    }
    if effective.kind == ScriptKind::RMarkdown
        && !effective.config.r.packages.iter().any(|package| {
            package
                .rsplit_once("::")
                .map(|(_, package)| package)
                .unwrap_or(package)
                .split('@')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("rmarkdown"))
        })
    {
        effective.config.r.packages.push("rmarkdown".to_owned());
    }
    let command = script_command(&effective.root, &effective.script, effective.kind)?;
    effective
        .config
        .tasks
        .insert(SCRIPT_TASK.to_owned(), TaskConfig::simple(command));
    Ok(())
}

fn script_command(root: &Path, script: &Path, kind: ScriptKind) -> Result<String> {
    let relative = script.strip_prefix(root).unwrap_or(script);
    let path = relative.to_string_lossy();
    if path.contains(['\n', '\r', '"', '\'', '$', '`']) {
        anyhow::bail!("script path contains characters that cannot be executed safely");
    }
    #[cfg(not(windows))]
    if path.contains('\\') {
        anyhow::bail!("script path contains characters that cannot be executed safely");
    }
    let quoted = format!("\"{path}\"");
    Ok(match kind {
        ScriptKind::Python => format!("python {quoted}"),
        ScriptKind::R => format!("Rscript {quoted}"),
        ScriptKind::Quarto => format!("quarto render {quoted}"),
        ScriptKind::RMarkdown => {
            let r_path = path.replace('\\', "/").replace('\'', "\\'");
            format!("Rscript -e \"rmarkdown::render('{r_path}')\"")
        }
    })
}

fn document_outputs(root: &Path, script: &Path) -> Vec<PathBuf> {
    let parent = script.parent().unwrap_or(root);
    let stem = script
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("document");
    ["html", "pdf", "docx", "ipynb"]
        .into_iter()
        .map(|extension| parent.join(format!("{stem}.{extension}")))
        .chain(std::iter::once(parent.join(format!("{stem}_files"))))
        .collect()
}

pub fn show_script_dependencies(path: &Path, json: bool) -> Result<()> {
    let effective = Config::for_script(path)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&DependencyReport::from_config(
                &effective.config,
                Some(effective.source.to_string())
            ))?
        );
    } else {
        print_dependencies(&effective.config, Some(&effective.source.to_string()));
    }
    Ok(())
}

pub fn show_project_dependencies(root: &Path, json: bool) -> Result<()> {
    let config = Config::load(root)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&DependencyReport::from_config(
                &config,
                Some("project dual.toml".to_owned())
            ))?
        );
    } else {
        print_dependencies(&config, Some("project dual.toml"));
    }
    Ok(())
}

#[derive(Serialize)]
struct DependencyReport {
    source: Option<String>,
    python: LanguageDependencies,
    r: RDependencies,
}

#[derive(Serialize)]
struct LanguageDependencies {
    enabled: bool,
    version: String,
    dependencies: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    indexes: Vec<String>,
}

#[derive(Serialize)]
struct RDependencies {
    enabled: bool,
    version: String,
    cran: Vec<String>,
    bioc: Vec<String>,
    github: Vec<String>,
}

impl DependencyReport {
    fn from_config(config: &Config, source: Option<String>) -> Self {
        let (cran, bioc, github) = grouped_r_packages(config);
        Self {
            source,
            python: LanguageDependencies {
                enabled: config.python.enabled,
                version: config.python.version.clone(),
                dependencies: config.python.packages.clone(),
                indexes: config
                    .python
                    .index
                    .iter()
                    .map(|index| index.url.clone())
                    .collect(),
            },
            r: RDependencies {
                enabled: config.r.enabled,
                version: config.r.version.clone(),
                cran,
                bioc,
                github,
            },
        }
    }
}

pub fn print_dependencies(config: &Config, source: Option<&str>) {
    if let Some(source) = source {
        println!("Source: {source}");
    }
    if config.python.enabled {
        println!("Python version: {}", config.python.version);
        print_list("Python dependencies", &config.python.packages);
        print_str_iter(
            "Python indexes",
            config.python.index.iter().map(|index| index.url.as_str()),
        );
    } else {
        println!("Python: not required");
    }
    if config.r.enabled {
        println!("R version: {}", config.r.version);
        let (cran, bioc, github) = grouped_r_packages(config);
        print_list("CRAN packages", &cran);
        print_list("Bioconductor packages", &bioc);
        print_list("GitHub packages", &github);
    } else {
        println!("R: not required");
    }
}

fn print_list(label: &str, values: &[String]) {
    if values.is_empty() {
        println!("{label}: (none)");
    } else {
        println!("{label}: {}", values.join(", "));
    }
}

fn print_str_iter<'a>(label: &str, mut values: impl Iterator<Item = &'a str>) {
    let Some(first) = values.next() else {
        println!("{label}: (none)");
        return;
    };

    print!("{label}: {first}");
    for value in values {
        print!(", {value}");
    }
    println!();
}

fn grouped_r_packages(config: &Config) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut cran = Vec::new();
    let mut bioc = Vec::new();
    let mut github = Vec::new();
    for package in &config.r.packages {
        if let Some(package) = package.strip_prefix("bioc::") {
            bioc.push(package.to_owned());
        } else if let Some(package) = package.strip_prefix("github::") {
            github.push(package.to_owned());
        } else if let Some(package) = package.strip_prefix("cran::") {
            cran.push(package.to_owned());
        } else {
            cran.push(package.clone());
        }
    }
    (cran, bioc, github)
}

#[derive(Clone, Copy, Debug)]
pub enum ExportFormat {
    Requirements,
    Renv,
    Dockerfile,
}

pub fn export(root: &Path, format: ExportFormat) -> Result<PathBuf> {
    let config = Config::load(root)?;
    let (path, contents) = match format {
        ExportFormat::Requirements => (root.join("requirements.txt"), python_requirements(&config)),
        ExportFormat::Renv => {
            let (cran, bioc, github) = grouped_r_packages(&config);
            let mut lines = vec![
                "# Generated by dual. Review before running.".to_owned(),
                "if (!requireNamespace(\"renv\", quietly = TRUE)) install.packages(\"renv\")"
                    .to_owned(),
                "renv::init(bare = TRUE)".to_owned(),
            ];
            if !cran.is_empty() {
                lines.push(format!("renv::install(c({}))", r_values(cran.iter())));
            }
            if !bioc.is_empty() {
                lines.push(
                    "if (!requireNamespace(\"BiocManager\", quietly = TRUE)) install.packages(\"BiocManager\")"
                        .to_owned(),
                );
                lines.push(format!(
                    "BiocManager::install(c({}))",
                    r_values(bioc.iter())
                ));
            }
            if !github.is_empty() {
                lines.push(format!(
                    "renv::install(c({}))",
                    r_values(github.iter().map(|package| format!("github::{package}")))
                ));
            }
            lines.push("renv::snapshot()".to_owned());
            (root.join("renv-dependencies.R"), lines.join("\n") + "\n")
        }
        ExportFormat::Dockerfile => {
            let python_version = docker_version(&config.python.version, "Python")?;
            let python_series = docker_series(&python_version);
            let r_version = docker_version(&config.r.version, "R")?;
            let base = if config.r.enabled {
                format!("rocker/r-ver:{r_version}")
            } else if config.python.enabled {
                format!("python:{python_version}-slim")
            } else {
                "debian:bookworm-slim".to_owned()
            };
            let system_packages = if config.r.enabled && config.python.enabled {
                format!(
                    "ARG DUAL_SYSTEM_PACKAGES=\"\"\nARG DUAL_PYTHON_SERIES={python_series}\nRUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv git build-essential ca-certificates $DUAL_SYSTEM_PACKAGES && rm -rf /var/lib/apt/lists/*\nRUN actual=\"$(python3 -c 'import sys; print(f\"{{sys.version_info.major}}.{{sys.version_info.minor}}\")')\" && case \"$actual\" in \"$DUAL_PYTHON_SERIES\"|\"$DUAL_PYTHON_SERIES\".*) ;; *) echo \"Configured Python $DUAL_PYTHON_SERIES is unavailable in the selected R base image (found $actual). Choose a compatible R image or install that Python version explicitly.\" >&2; exit 1 ;; esac\nRUN python3 -m venv /opt/dual-python\nENV PATH=\"/opt/dual-python/bin:${{PATH}}\"\n"
                )
            } else {
                "ARG DUAL_SYSTEM_PACKAGES=\"\"\nRUN apt-get update && apt-get install -y --no-install-recommends git build-essential ca-certificates $DUAL_SYSTEM_PACKAGES && rm -rf /var/lib/apt/lists/*\n".to_owned()
            };
            let python_install = if config.python.packages.is_empty() {
                String::new()
            } else {
                format!(
                    "RUN <<'EOF'\ncat > /tmp/requirements.txt <<'REQ'\n{}REQ\npython -m pip install --no-cache-dir -r /tmp/requirements.txt\nEOF\n",
                    python_requirements(&config)
                )
            };
            let r_packages = config
                .r
                .packages
                .iter()
                .map(|package| {
                    if package.contains("::") {
                        package.clone()
                    } else {
                        format!("cran::{package}")
                    }
                })
                .collect::<Vec<_>>();
            let r_layer = if r_packages.is_empty() {
                String::new()
            } else {
                format!(
                    "RUN Rscript -e \"options(repos=c(CRAN='https://cloud.r-project.org')); install.packages('pak'); pak::pkg_install(c({}))\"\n",
                    r_single_values(r_packages.iter())
                )
            };
            let quarto = if config.quarto.enabled {
                "# Quarto is enabled in dual.toml. Add a pinned Quarto release here if your image must render documents.\n"
            } else {
                ""
            };
            ensure_dockerignore(root)?;
            (
                root.join("Dockerfile"),
                format!(
                    "# syntax=docker/dockerfile:1\n# Generated by dual. Review versions and system libraries before production use.\n# Add OS packages required by compiled dependencies with:\n#   docker build --build-arg DUAL_SYSTEM_PACKAGES=\"libcurl4-openssl-dev libssl-dev libxml2-dev\" .\nFROM {base}\nLABEL org.opencontainers.image.source=\"dual\"\nSHELL [\"/bin/bash\", \"-euo\", \"pipefail\", \"-c\"]\n{system_packages}WORKDIR /project\n{python_install}{r_layer}{quarto}COPY . /project\nCMD [\"bash\"]\n",
                ),
            )
        }
    };
    security::write_file_atomic(&path, contents.as_bytes(), "export file")?;
    Ok(path)
}

fn python_requirements(config: &Config) -> String {
    let mut lines = Vec::new();
    for (position, index) in config.python.index.iter().enumerate() {
        let option = if position == 0 {
            "--index-url"
        } else {
            "--extra-index-url"
        };
        lines.push(format!("{option} {}", index.url));
    }
    lines.extend(config.python.packages.iter().cloned());
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

fn ensure_dockerignore(root: &Path) -> Result<()> {
    const REQUIRED: &[&str] = &[
        ".dual/",
        "target/",
        ".git/",
        "results/",
        ".env",
        ".env.*",
        "!.env.example",
    ];

    let path = root.join(".dockerignore");
    security::reject_symlink_if_present(&path, ".dockerignore")?;
    let mut contents = if path.try_exists()? {
        security::read_text_file(&path, MAX_CONFIG_BYTES, ".dockerignore")?
    } else {
        String::new()
    };
    let existing = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<std::collections::BTreeSet<_>>();
    let missing = REQUIRED
        .iter()
        .copied()
        .filter(|entry| !existing.contains(entry))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    if !contents.is_empty() {
        contents.push('\n');
    }
    contents.push_str("# Added by dual; local environments and secrets stay outside the image.\n");
    for entry in missing {
        contents.push_str(entry);
        contents.push('\n');
    }
    security::write_file_atomic(&path, contents.as_bytes(), ".dockerignore")
}

fn r_values(values: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let mut rendered = String::new();
    for value in values {
        if !rendered.is_empty() {
            rendered.push_str(", ");
        }
        rendered.push('"');
        for character in value.as_ref().chars() {
            match character {
                '\\' => rendered.push_str("\\\\"),
                '"' => rendered.push_str("\\\""),
                _ => rendered.push(character),
            }
        }
        rendered.push('"');
    }
    rendered
}

fn r_single_values(values: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    values
        .into_iter()
        .map(|value| format!("'{}'", escape_single(value.as_ref())))
        .collect::<Vec<_>>()
        .join(", ")
}

fn escape_single(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn docker_version(value: &str, language: &str) -> Result<String> {
    let constraint = value.split(',').next().unwrap_or(value).trim();
    if constraint == "*"
        || constraint.contains('*')
        || constraint.starts_with('<')
        || constraint.starts_with('!')
        || (constraint.starts_with('>') && !constraint.starts_with(">="))
    {
        anyhow::bail!(
            "Docker export cannot choose a {language} image from the ambiguous version requirement {value:?}; use an exact version or a lower bound such as `>=3.12`"
        );
    }
    let version = constraint.trim_start_matches(['>', '=', '~', '^']).trim();
    if version.is_empty() {
        anyhow::bail!("Docker export cannot derive a {language} image version from {value:?}");
    }
    Ok(version.to_owned())
}

fn docker_series(value: &str) -> String {
    value.split('.').take(2).collect::<Vec<_>>().join(".")
}

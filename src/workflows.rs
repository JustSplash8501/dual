use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::backend::{Backend, EnvironmentBackend};
use crate::config::{Config, EffectiveConfig};
use crate::metadata::ScriptKind;
use crate::security;

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
        println!("Would run: {command}");
        return Ok(());
    }

    let backend = EnvironmentBackend::for_script(&effective.root, &effective.script, verbose);
    let trust = security::ensure_project_trusted(&effective.root, trust_project)?;
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
        backend.validate(&effective.config)?;
        security::verify_project_unchanged(&effective.root, &trust)?;
        security::refresh_project_trust(&effective.root)?;
    }
    println!("Running {}...", effective.script.display());
    let document_snapshot = matches!(effective.kind, ScriptKind::Quarto | ScriptKind::RMarkdown)
        .then(|| {
            security::snapshot_project_excluding(
                &effective.root,
                &document_outputs(&effective.root, &effective.script),
            )
        })
        .transpose()?;
    backend.run(&effective.config, SCRIPT_TASK)?;
    if let Some(snapshot) = document_snapshot {
        security::verify_project_snapshot(&effective.root, &snapshot)
    } else {
        security::verify_project_unchanged(&effective.root, &trust)
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
    backend.validate(&effective.config)?;
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
    sync_config(root, &config, verbose, trust_project, true)?;
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
    backend.validate(config)?;
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
        .insert(SCRIPT_TASK.to_owned(), command);
    Ok(())
}

fn script_command(root: &Path, script: &Path, kind: ScriptKind) -> Result<String> {
    let relative = script.strip_prefix(root).unwrap_or(script);
    let path = relative.to_string_lossy();
    if path.contains(['\n', '\r', '"']) {
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

pub fn show_script_dependencies(path: &Path) -> Result<()> {
    let effective = Config::for_script(path)?;
    print_dependencies(&effective.config, Some(&effective.source.to_string()));
    Ok(())
}

pub fn show_project_dependencies(root: &Path) -> Result<()> {
    let config = Config::load(root)?;
    print_dependencies(&config, Some("project dual.toml"));
    Ok(())
}

pub fn print_dependencies(config: &Config, source: Option<&str>) {
    if let Some(source) = source {
        println!("Source: {source}");
    }
    if config.python.enabled {
        println!("Python version: {}", config.python.version);
        print_list("Python dependencies", &config.python.packages);
        let indexes = config
            .python
            .index
            .iter()
            .map(|index| index.url.clone())
            .collect::<Vec<_>>();
        print_list("Python indexes", &indexes);
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
        ExportFormat::Requirements => (
            root.join("requirements.txt"),
            if config.python.packages.is_empty() {
                String::new()
            } else {
                config.python.packages.join("\n") + "\n"
            },
        ),
        ExportFormat::Renv => {
            let (cran, bioc, github) = grouped_r_packages(&config);
            let mut lines = vec![
                "# Generated by dual. Review before running.".to_owned(),
                "if (!requireNamespace(\"renv\", quietly = TRUE)) install.packages(\"renv\")"
                    .to_owned(),
                "renv::init(bare = TRUE)".to_owned(),
            ];
            if !cran.is_empty() {
                lines.push(format!("renv::install(c({}))", r_values(&cran)));
            }
            if !bioc.is_empty() {
                lines.push(
                    "if (!requireNamespace(\"BiocManager\", quietly = TRUE)) install.packages(\"BiocManager\")"
                        .to_owned(),
                );
                lines.push(format!("BiocManager::install(c({}))", r_values(&bioc)));
            }
            if !github.is_empty() {
                lines.push(format!(
                    "renv::install(c({}))",
                    r_values(
                        &github
                            .iter()
                            .map(|package| format!("github::{package}"))
                            .collect::<Vec<_>>()
                    )
                ));
            }
            lines.push("renv::snapshot()".to_owned());
            (root.join("renv-dependencies.R"), lines.join("\n") + "\n")
        }
        ExportFormat::Dockerfile => {
            let requirements = if config.python.packages.is_empty() {
                String::new()
            } else {
                "COPY requirements.txt /tmp/requirements.txt\nRUN python -m pip install --no-cache-dir -r /tmp/requirements.txt\n".to_owned()
            };
            let (cran, bioc, github) = grouped_r_packages(&config);
            let mut r_install = cran
                .iter()
                .map(|package| format!("install.packages('{}')", escape_single(package)))
                .collect::<Vec<_>>();
            r_install.extend(bioc.iter().map(|package| {
                format!(
                    "BiocManager::install('{}', ask=FALSE)",
                    escape_single(package)
                )
            }));
            r_install.extend(
                github
                    .iter()
                    .map(|package| format!("pak::pkg_install('{}')", escape_single(package))),
            );
            let r_layer = if r_install.is_empty() {
                String::new()
            } else {
                format!(
                    "RUN Rscript -e \"install.packages(c('pak','BiocManager')); {}\"\n",
                    r_install.join("; ")
                )
            };
            let quarto = if config.quarto.enabled {
                "# Quarto documents require adding the appropriate Quarto release for this image.\n"
            } else {
                ""
            };
            (
                root.join("Dockerfile"),
                format!(
                    "# Generated by dual. Review versions and system libraries before production use.\nFROM rocker/r-ver:{}\nRUN apt-get update && apt-get install -y --no-install-recommends python3 python3-pip git build-essential && rm -rf /var/lib/apt/lists/*\nWORKDIR /project\n{requirements}{r_layer}{quarto}COPY . /project\nCMD [\"bash\"]\n",
                    config.r.version.trim_start_matches(['>', '=', '<', '~', '^'])
                ),
            )
        }
    };
    security::write_file_atomic(&path, contents.as_bytes(), "export file")?;
    Ok(path)
}

fn r_values(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ")
}

fn escape_single(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

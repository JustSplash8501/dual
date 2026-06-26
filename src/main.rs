use anyhow::Result;
use clap::Parser;
use dual::backend::{Backend, EnvironmentBackend};
use dual::cli::{Cli, Commands, EngineCommand, Language, LockCommand, TaskCommand};
use dual::config::{validate_project_name, Config, DEFAULT_CONFIG};
use dual::imports;
use dual::metadata::{self, AddOptions, ScriptLanguage};
use dual::workflows::{self, ExportFormat};
use dual::{doctor, security, tasks};
use std::path::Path;

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let current = std::env::current_dir()?;
    let verbose = cli.verbose;
    let trust_project = cli.trust_project;
    let json = cli.json;

    match cli.command {
        Commands::Init {
            force,
            name,
            legacy_name,
            script,
            python,
            r,
        } => {
            if let Some(script) = script {
                if name.is_some() || legacy_name.is_some() {
                    anyhow::bail!("a project name cannot be used with `dual init --script`");
                }
                let script = metadata::absolute_path(&script)?;
                metadata::initialize(&script, python.as_deref(), r.as_deref(), force)?;
                println!("Initialized inline metadata in {}.", script.display());
                return Ok(());
            }
            if python.is_some() || r.is_some() {
                anyhow::bail!("`--python` and `--r` require `dual init --script FILE`");
            }
            if name.is_some() && legacy_name.is_some() {
                anyhow::bail!("provide the project name once: `dual init PROJECT_NAME`");
            }
            init(&current, force, name.as_deref().or(legacy_name.as_deref()))
        }
        Commands::Add {
            script,
            python,
            r,
            index,
            github,
            bioc,
            items,
        } => {
            if let Some(script) = script {
                let language = match (python, r) {
                    (true, false) => Some(ScriptLanguage::Python),
                    (false, true) => Some(ScriptLanguage::R),
                    (false, false) => None,
                    (true, true) => unreachable!("clap rejects conflicting flags"),
                };
                let script = metadata::absolute_path(&script)?;
                metadata::add(
                    &script,
                    AddOptions {
                        language,
                        packages: &items,
                        index: index.as_deref(),
                        github: github.as_deref(),
                        bioc,
                    },
                )?;
                println!("Updated inline metadata in {}.", script.display());
                return Ok(());
            }
            if python || r || index.is_some() || github.is_some() || bioc {
                anyhow::bail!("script package-source flags require `dual add --script FILE`");
            }
            let (language, packages) = parse_project_add_items(&items)?;
            let root = Config::find_root(&current)?;
            add(&root, language, packages)
        }
        Commands::Remove { language, packages } => {
            let root = Config::find_root(&current)?;
            remove(&root, language, &packages)
        }
        Commands::Import { file } => {
            let root = Config::find_root(&current)?;
            import(&root, &file, json)
        }
        Commands::Up { refresh } => {
            let root = Config::find_root(&current)?;
            let backend = EnvironmentBackend::new(&root, verbose);
            up(&root, &backend, refresh, trust_project)
        }
        Commands::Run {
            target,
            no_install,
            dry_run,
        } => {
            if workflows::looks_like_script(&target) {
                workflows::run_script(
                    Path::new(&target),
                    verbose,
                    trust_project,
                    no_install,
                    dry_run,
                )
            } else {
                if no_install || dry_run {
                    anyhow::bail!("`--no-install` and `--dry-run` are supported for script runs");
                }
                let root = Config::find_root(&current)?;
                let backend = EnvironmentBackend::new(&root, verbose);
                tasks::run_task(&root, &backend, &target, trust_project)
            }
        }
        Commands::Sync { script, dry_run } => {
            if let Some(script) = script {
                workflows::sync_script(&script, verbose, trust_project, dry_run)
            } else {
                let root = Config::find_root(&current)?;
                workflows::sync_project(&root, verbose, trust_project, dry_run)
            }
        }
        Commands::Deps { script } => {
            if let Some(script) = script {
                workflows::show_script_dependencies(&script, json)
            } else {
                let root = Config::find_root(&current)?;
                workflows::show_project_dependencies(&root, json)
            }
        }
        Commands::Export {
            requirements,
            renv,
            dockerfile: _,
        } => {
            let root = Config::find_root(&current)?;
            let format = if requirements {
                ExportFormat::Requirements
            } else if renv {
                ExportFormat::Renv
            } else {
                ExportFormat::Dockerfile
            };
            let path = workflows::export(&root, format)?;
            println!("Wrote {}.", path.display());
            Ok(())
        }
        Commands::Task {
            command: TaskCommand::List,
        } => {
            let root = Config::find_root(&current)?;
            tasks::list_tasks(&root, json)
        }
        Commands::Engine {
            command: EngineCommand::Update,
        } => EnvironmentBackend::new(&current, verbose).update_engine(),
        Commands::Engine {
            command: EngineCommand::Uninstall,
        } => {
            let backend = EnvironmentBackend::new(&current, verbose);
            if backend.uninstall_engine()? {
                println!("Removed dual's private environment support.");
            } else {
                println!("No private environment support was installed.");
            }
            Ok(())
        }
        Commands::Lock {
            command: LockCommand::Migrate,
        } => {
            let root = Config::find_root(&current)?;
            let backend = EnvironmentBackend::new(&root, verbose);
            if backend.migrate_lock()? {
                println!("Migrated dual.lock to the current format.");
            } else {
                println!("dual.lock is already current.");
            }
            Ok(())
        }
        Commands::Shell => {
            let root = Config::find_root(&current)?;
            let backend = EnvironmentBackend::new(&root, verbose);
            shell(&root, &backend, trust_project)
        }
        Commands::Doctor => {
            let root = Config::find_root_optional(&current);
            if let Some(root) = root {
                let backend = EnvironmentBackend::new(&root, verbose);
                if backend.environment_exists() {
                    let trust = security::ensure_project_trusted(&root, trust_project)?;
                    let config = Config::load(&root)?;
                    backend.verify_manifest(&config)?;
                    security::verify_project_unchanged(&root, &trust)?;
                }
                doctor::run(&root, &backend, json)
            } else {
                doctor::run_system(json)
            }
        }
        Commands::Clean { yes } => {
            let root = Config::find_root(&current)?;
            let backend = EnvironmentBackend::new(&root, verbose);
            clean(&root, &backend, yes)
        }
    }
}

fn import(root: &std::path::Path, file: &std::path::Path, json: bool) -> Result<()> {
    security::reject_symlink_if_present(&Config::path(root), "dual.toml")?;
    let preserve_trust = security::project_is_trusted(root)?;
    let report = imports::import_file(root, file)?;
    if preserve_trust {
        security::refresh_project_trust(root)?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Imported dependencies from {}.", report.source);
        if let Some(version) = &report.python_version {
            println!("Python version: {version}");
        }
        if !report.python.is_empty() {
            println!("Python packages: {}", report.python.join(", "));
        }
        if let Some(version) = &report.r_version {
            println!("R version: {version}");
        }
        if !report.r.is_empty() {
            println!("R packages: {}", report.r.join(", "));
        }
        if !report.skipped.is_empty() {
            println!("Skipped unsupported entries: {}", report.skipped.join(", "));
        }
        if root.join("dual.lock").is_file() {
            println!("Run `dual up --refresh` to update the shared environment lock.");
        }
    }
    Ok(())
}

fn parse_project_add_items(items: &[String]) -> Result<(Language, &[String])> {
    let Some((language, packages)) = items.split_first() else {
        anyhow::bail!("usage: dual add <r|py> PACKAGE...");
    };
    if packages.is_empty() {
        anyhow::bail!("provide at least one package name");
    }
    let language = match language.to_ascii_lowercase().as_str() {
        "r" => Language::R,
        "py" | "python" => Language::Py,
        _ => anyhow::bail!("package ecosystem must be `r` or `py`"),
    };
    Ok((language, packages))
}

fn remove(root: &std::path::Path, language: Language, packages: &[String]) -> Result<()> {
    security::reject_symlink_if_present(&Config::path(root), "dual.toml")?;
    let preserve_trust = security::project_is_trusted(root)?;
    let section = match language {
        Language::R => "r",
        Language::Py => "python",
    };
    let removed = Config::remove_packages(&Config::path(root), section, packages)?;
    if preserve_trust {
        security::refresh_project_trust(root)?;
    }
    println!("Removed {removed} package(s) from dual.toml.");
    if root.join("dual.lock").is_file() {
        println!("Run `dual up --refresh` to update the shared environment lock.");
    }
    Ok(())
}

fn init(root: &std::path::Path, force: bool, name: Option<&str>) -> Result<()> {
    let inferred_name = root
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| anyhow::anyhow!("could not infer a project name from this directory"))?;
    let project_name = name.unwrap_or(inferred_name);
    validate_project_name(project_name)?;

    let path = Config::path(root);
    security::reject_symlink_if_present(&path, "dual.toml")?;
    if path.exists() && !force {
        anyhow::bail!("dual.toml already exists. Use `dual init --force` to replace it.");
    }
    if force {
        let state = root.join(".dual");
        let lock = root.join("dual.lock");
        security::ensure_managed_path(root, &state)?;
        security::reject_symlink_if_present(&lock, "dual.lock")?;
        if state.is_dir() {
            std::fs::remove_dir_all(&state)?;
        }
        if lock.is_file() {
            std::fs::remove_file(&lock)?;
        }
        println!("Invalidated the previous environment and lockfile.");
    }

    let escaped_name = project_name.replace('\\', "\\\\").replace('"', "\\\"");
    let contents = DEFAULT_CONFIG.replace(
        "name = \"my-project\"",
        &format!("name = \"{escaped_name}\""),
    );
    security::write_file_atomic(&path, contents.as_bytes(), "dual.toml")?;
    for directory in ["scripts", "data", "results"] {
        std::fs::create_dir_all(root.join(directory))?;
    }

    println!("Created dual.toml");
    println!("Created scripts/, data/, and results/");
    println!("Next: add packages, then run `dual up`.");
    Ok(())
}

fn add(root: &std::path::Path, language: Language, packages: &[String]) -> Result<()> {
    security::reject_symlink_if_present(&Config::path(root), "dual.toml")?;
    let preserve_trust = security::project_is_trusted(root)?;
    let path = Config::path(root);
    let section = match language {
        Language::R => "r",
        Language::Py => "python",
    };
    Config::add_packages(&path, section, packages)?;
    if preserve_trust {
        security::refresh_project_trust(root)?;
    }

    let label = match language {
        Language::R => "R",
        Language::Py => "Python",
    };
    println!(
        "Added {} {} package(s) to dual.toml.",
        packages.len(),
        label
    );
    if root.join("dual.lock").is_file() {
        println!("Run `dual up --refresh` to update the shared environment lock.");
    } else {
        println!("Run `dual up` to create the project environment.");
    }
    Ok(())
}

fn up(
    root: &std::path::Path,
    backend: &impl Backend,
    refresh: bool,
    trust_project: bool,
) -> Result<()> {
    let config = Config::load(root)?;
    let trust = security::ensure_project_trusted(root, trust_project)?;

    println!("Preparing project environment...");
    println!("✓ R {} requested", config.r.version);
    println!("✓ Python {} requested", config.python.version);

    backend.ensure_available()?;
    security::verify_project_unchanged(root, &trust)?;

    backend.init_or_update(&config, refresh)?;
    println!("✓ R packages configured");
    println!("✓ Python packages configured");

    backend.validate(&config)?;
    security::verify_project_unchanged(root, &trust)?;
    security::refresh_project_trust(root)?;
    println!("✓ Project is ready");
    Ok(())
}

fn shell(root: &std::path::Path, backend: &impl Backend, trust_project: bool) -> Result<()> {
    let config = Config::load(root)?;
    if !backend.environment_exists() {
        anyhow::bail!("The project environment has not been created. Run `dual up` first.");
    }
    let trust = security::ensure_project_trusted(root, trust_project)?;
    backend.ensure_available()?;
    backend.verify_manifest(&config)?;
    security::verify_project_unchanged(root, &trust)?;
    backend.shell(&config)?;
    security::verify_project_unchanged(root, &trust)
}

fn clean(root: &std::path::Path, backend: &impl Backend, yes: bool) -> Result<()> {
    Config::load(root)?;
    if !yes && !confirm_clean()? {
        println!("Clean cancelled.");
        return Ok(());
    }

    let removed = backend.clean()?;
    if removed.is_empty() {
        println!("Nothing to clean.");
    } else {
        println!("Removed generated environment files:");
        for path in removed {
            println!("  {}", display_relative(root, &path));
        }
    }
    Ok(())
}

fn confirm_clean() -> Result<bool> {
    use std::io::{self, Write};

    print!("Remove dual's generated environment files? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn display_relative(root: &std::path::Path, path: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

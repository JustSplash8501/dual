use anyhow::Result;
use clap::Parser;
use dual::backend::{Backend, EnvironmentBackend};
use dual::cli::{Cli, Commands, EngineCommand, Language, LockCommand, TaskCommand};
use dual::config::{validate_project_name, Config, DEFAULT_CONFIG};
use dual::{doctor, security, tasks};

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let current = std::env::current_dir()?;
    let root = if matches!(
        &cli.command,
        Commands::Init { .. } | Commands::Engine { .. }
    ) {
        current
    } else {
        Config::find_root(&current)?
    };
    let backend = EnvironmentBackend::new(&root, cli.verbose);

    match cli.command {
        Commands::Init {
            force,
            name,
            legacy_name,
        } => {
            if name.is_some() && legacy_name.is_some() {
                anyhow::bail!("provide the project name once: `dual init PROJECT_NAME`");
            }
            init(&root, force, name.as_deref().or(legacy_name.as_deref()))
        }
        Commands::Add { language, packages } => add(&root, language, &packages),
        Commands::Remove { language, packages } => remove(&root, language, &packages),
        Commands::Up { refresh } => up(&root, &backend, refresh, cli.trust_project),
        Commands::Run { task } => tasks::run_task(&root, &backend, &task, cli.trust_project),
        Commands::Task {
            command: TaskCommand::List,
        } => tasks::list_tasks(&root),
        Commands::Engine {
            command: EngineCommand::Update,
        } => backend.update_engine(),
        Commands::Engine {
            command: EngineCommand::Uninstall,
        } => {
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
            if backend.migrate_lock()? {
                println!("Migrated dual.lock to the current format.");
            } else {
                println!("dual.lock is already current.");
            }
            Ok(())
        }
        Commands::Shell => shell(&root, &backend, cli.trust_project),
        Commands::Doctor => {
            if backend.environment_exists() {
                let trust = security::ensure_project_trusted(&root, cli.trust_project)?;
                let config = Config::load(&root)?;
                backend.verify_manifest(&config)?;
                security::verify_project_unchanged(&root, &trust)?;
            }
            doctor::run(&root, &backend)
        }
        Commands::Clean { yes } => clean(&root, &backend, yes),
    }
}

fn remove(root: &std::path::Path, language: Language, packages: &[String]) -> Result<()> {
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

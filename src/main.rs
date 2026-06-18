use anyhow::Result;
use clap::Parser;
use dual::backend::{Backend, EnvironmentBackend};
use dual::cli::{Cli, Commands, Language};
use dual::config::{Config, DEFAULT_CONFIG};
use dual::{doctor, tasks};

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let current = std::env::current_dir()?;
    let root = if matches!(cli.command, Commands::Init { .. }) {
        current
    } else {
        Config::find_root(&current)?
    };
    let backend = EnvironmentBackend::new(&root, cli.verbose);

    match cli.command {
        Commands::Init { force, name } => init(&root, force, name.as_deref()),
        Commands::Add { language, packages } => add(&root, language, &packages),
        Commands::Up { refresh } => up(&root, &backend, refresh),
        Commands::Run { task } => tasks::run_task(&root, &backend, &task),
        Commands::Shell => shell(&root, &backend),
        Commands::Doctor => doctor::run(&root, &backend),
        Commands::Clean { yes } => clean(&root, &backend, yes),
    }
}

fn init(root: &std::path::Path, force: bool, name: Option<&str>) -> Result<()> {
    let path = Config::path(root);
    if path.exists() && !force {
        anyhow::bail!("dual.toml already exists. Use `dual init --force` to replace it.");
    }
    if force {
        let state = root.join(".dual");
        let lock = root.join("dual.lock");
        if state.is_dir() {
            std::fs::remove_dir_all(&state)?;
        }
        if lock.is_file() {
            std::fs::remove_file(&lock)?;
        }
        println!("Invalidated the previous environment and lockfile.");
    }

    let project_name = name.unwrap_or("my-project");

    let escaped_name = project_name.replace('\\', "\\\\").replace('"', "\\\"");
    let contents = DEFAULT_CONFIG.replace(
        "name = \"my-project\"",
        &format!("name = \"{escaped_name}\""),
    );
    std::fs::write(&path, contents)?;
    for directory in ["scripts", "data", "results"] {
        std::fs::create_dir_all(root.join(directory))?;
    }

    println!("Created dual.toml");
    println!("Created scripts/, data/, and results/");
    println!("Next: add packages, then run `dual up`.");
    Ok(())
}

fn add(root: &std::path::Path, language: Language, packages: &[String]) -> Result<()> {
    let path = Config::path(root);
    let section = match language {
        Language::R => "r",
        Language::Py => "python",
    };
    Config::add_packages(&path, section, packages)?;

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

fn up(root: &std::path::Path, backend: &impl Backend, refresh: bool) -> Result<()> {
    let config = Config::load(root)?;

    println!("Preparing project environment...");
    println!("✓ R {} requested", config.r.version);
    println!("✓ Python {} requested", config.python.version);

    backend.ensure_available()?;

    backend.init_or_update(&config, refresh)?;
    println!("✓ R packages configured");
    println!("✓ Python packages configured");

    backend.validate(&config)?;
    println!("✓ Project is ready");
    Ok(())
}

fn shell(root: &std::path::Path, backend: &impl Backend) -> Result<()> {
    Config::load(root)?;
    backend.ensure_available()?;
    backend.shell()
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

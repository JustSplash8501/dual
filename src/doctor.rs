use std::path::Path;

use anyhow::Result;

use crate::backend::Backend;
use crate::config::Config;
use crate::platform;

pub fn run(root: &Path, backend: &impl Backend) -> Result<()> {
    println!("Project");
    let config_path = Config::path(root);
    if !config_path.exists() {
        println!("✗ dual.toml not found");
        println!("\nSuggested fixes:\n  dual init");
        return Ok(());
    }
    println!("✓ dual.toml found");

    let config = match Config::load(root) {
        Ok(config) => {
            println!("✓ config is valid");
            config
        }
        Err(error) => {
            println!("✗ config is invalid: {error}");
            println!("\nSuggested fixes:\n  Correct dual.toml, then run `dual doctor` again");
            return Ok(());
        }
    };

    let report = backend.doctor(&config)?;
    let mut fixes = Vec::new();

    println!("\nEnvironment");
    if report.available {
        println!("✓ environment support available");
    } else {
        println!("⚠ environment support will be installed automatically");
        fixes.push("dual up");
    }
    if report.environment_present {
        println!("✓ project environment present");
    } else {
        println!("⚠ project environment has not been created");
        fixes.push("dual up");
    }
    if report.lock_present {
        println!("✓ shared lockfile present");
    } else {
        println!("⚠ shared lockfile has not been created");
        fixes.push("dual up");
    }

    println!("\nR");
    print_runtime("R", report.r_available);
    println!("✓ {} R packages configured", config.r.packages.len());
    for package in &report.missing_r_packages {
        println!("⚠ Package not installed: {package}");
        fixes.push("dual up");
    }

    println!("\nPython");
    print_runtime("Python", report.python_available);
    println!(
        "✓ {} Python packages configured",
        config.python.packages.len()
    );
    for package in &report.missing_python_packages {
        println!("⚠ Package not installed: {package}");
        fixes.push("dual up");
    }

    println!("\nBridge");
    match report.bridge {
        Some(bridge) => {
            println!("✓ R and Python are both enabled");
            if !bridge.reticulate_installed {
                println!("⚠ reticulate is not installed");
                fixes.push("dual add r reticulate");
            } else if bridge.uses_project_python {
                println!("✓ reticulate uses the project Python");
            } else {
                println!("⚠ reticulate is not using the project Python");
                fixes.push("dual up");
            }
        }
        None if report.environment_present => {
            println!("⚠ R/Python bridge could not be checked");
        }
        None => println!("⚠ Create the environment to check R/Python interoperability"),
    }

    println!("\nTasks");
    if config.tasks.is_empty() {
        println!("⚠ no tasks configured");
    } else {
        for (name, command) in &config.tasks {
            println!("✓ {name} task configured");
            if let Some(script) = platform::referenced_script(command) {
                if !root.join(&script).exists() {
                    println!("✗ {} does not exist", script.display());
                }
            }
        }
    }

    fixes.sort_unstable();
    fixes.dedup();
    if !fixes.is_empty() {
        println!("\nSuggested fixes:");
        for fix in fixes {
            println!("  {fix}");
        }
    }
    Ok(())
}

fn print_runtime(name: &str, status: Option<bool>) {
    match status {
        Some(true) => println!("✓ {name} available"),
        Some(false) => println!("✗ {name} is not available inside the project environment"),
        None => println!("⚠ {name} not checked"),
    }
}

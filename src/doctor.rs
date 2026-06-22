use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::backend::Backend;
use crate::config::Config;
use crate::platform;

pub fn run(root: &Path, backend: &impl Backend) -> Result<()> {
    print_system_checks();
    println!();
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
    if config.r.enabled {
        print_runtime("R", report.r_available);
        println!("✓ {} R packages configured", config.r.packages.len());
        for package in &report.missing_r_packages {
            println!("⚠ Package not installed: {package}");
            fixes.push("dual up");
        }
    } else {
        println!("○ R is not required by this environment");
    }

    println!("\nPython");
    if config.python.enabled {
        print_runtime("Python", report.python_available);
        println!(
            "✓ {} Python packages configured",
            config.python.packages.len()
        );
        for package in &report.missing_python_packages {
            println!("⚠ Package not installed: {package}");
            fixes.push("dual up");
        }
    } else {
        println!("○ Python is not required by this environment");
    }

    println!("\nBridge");
    if !config.r.enabled || !config.python.enabled {
        println!("○ R/Python bridge is not required");
    } else {
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

pub fn run_system() -> Result<()> {
    print_system_checks();
    println!("\nProject");
    println!("⚠ no dual.toml found from the current directory");
    println!("\nSuggested fixes:\n  dual init");
    Ok(())
}

fn print_system_checks() {
    println!("System");
    println!(
        "✓ operating system: {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    print_tool("Python", &["python", "python3"]);
    print_tool("uv", &["uv"]);
    print_tool("R", &["R"]);
    print_tool("Rscript", &["Rscript"]);
    print_tool("Quarto", &["quarto"]);
    print_tool("Git", &["git"]);
    let build_tools = if cfg!(windows) {
        &["cl", "gcc", "clang"][..]
    } else {
        &["cc", "gcc", "clang", "make"][..]
    };
    if build_tools.iter().any(|tool| tool_available(tool)) {
        println!("✓ compiler or build tool available");
    } else {
        println!("⚠ no common compiler or build tool found on PATH");
    }
}

fn print_tool(label: &str, candidates: &[&str]) {
    if let Some(tool) = candidates.iter().find(|tool| tool_available(tool)) {
        println!("✓ {label} available ({tool})");
    } else {
        println!("⚠ {label} not found on PATH");
    }
}

fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn print_runtime(name: &str, status: Option<bool>) {
    match status {
        Some(true) => println!("✓ {name} available"),
        Some(false) => println!("✗ {name} is not available inside the project environment"),
        None => println!("⚠ {name} not checked"),
    }
}

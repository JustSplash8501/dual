use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "dual",
    version,
    about = "Reproducible R and Python projects without environment-tool busywork",
    long_about = "dual coordinates a reproducible project environment for R, Python, or both. \
                  Edit dual.toml; dual handles the environment."
)]
pub struct Cli {
    /// Show additional environment progress.
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Trust the current project contents to install packages or execute code.
    #[arg(long, global = true)]
    pub trust_project: bool,

    /// Emit machine-readable JSON for supported commands.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Create dual.toml or initialize inline script metadata.
    Init {
        /// Replace an existing dual.toml.
        #[arg(long)]
        force: bool,

        /// Project name used in dual.toml and the activated shell prompt.
        #[arg(value_name = "PROJECT_NAME")]
        name: Option<String>,

        /// Legacy spelling for the project name.
        #[arg(long = "name", value_name = "PROJECT_NAME", hide = true)]
        legacy_name: Option<String>,

        /// Create or update inline metadata in a script or document.
        #[arg(long, value_name = "FILE")]
        script: Option<PathBuf>,

        /// Include Python at this version in a project or script.
        #[arg(long, value_name = "VERSION")]
        python: Option<String>,

        /// Include R at this version in a project or script.
        #[arg(long, value_name = "VERSION")]
        r: Option<String>,
    },

    /// Add packages to dual.toml or inline script metadata.
    Add {
        /// Add packages to inline script metadata instead of dual.toml.
        #[arg(long, value_name = "FILE")]
        script: Option<PathBuf>,

        /// Add packages as Python dependencies.
        #[arg(long, conflicts_with = "r")]
        python: bool,

        /// Add packages as R dependencies.
        #[arg(long, conflicts_with = "python")]
        r: bool,

        /// Add a Python package index.
        #[arg(long, value_name = "URL")]
        index: Option<String>,

        /// Add an R package from OWNER/REPO on GitHub.
        #[arg(long, value_name = "OWNER/REPO")]
        github: Option<String>,

        /// Add R packages from Bioconductor instead of CRAN.
        #[arg(long)]
        bioc: bool,

        /// For projects: r or py followed by packages. For scripts: packages.
        #[arg(num_args = 0.., value_name = "PACKAGE")]
        items: Vec<String>,
    },

    /// Remove packages from dual.toml.
    Remove {
        /// Package ecosystem: r or py.
        #[arg(value_enum)]
        language: Language,

        /// One or more package names.
        #[arg(required = true, num_args = 1..)]
        packages: Vec<String>,
    },

    /// Enable an R or Python runtime in dual.toml.
    Enable {
        /// Runtime to enable: r or py.
        #[arg(value_enum)]
        language: Language,

        /// Runtime version. Uses Dual's default when enabling an omitted runtime.
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Disable an R or Python runtime in dual.toml.
    Disable {
        /// Runtime to disable: r or py.
        #[arg(value_enum)]
        language: Language,

        /// Remove configured packages and indexes with the runtime.
        #[arg(long)]
        force: bool,
    },

    /// Import dependencies from an existing environment or lock file.
    Import {
        /// pyproject.toml, requirements.txt, renv.lock, env.lock, uv.lock, or environment.yml.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    /// Create or update the project environment.
    Up {
        /// Re-resolve dependencies and update dual.lock.
        #[arg(long)]
        refresh: bool,
    },

    /// Run a configured task or supported script file.
    Run {
        /// Task name from [tasks], or a .py, .R, .qmd, or .Rmd file.
        target: String,

        /// Arguments passed to the script or task after `--`.
        #[arg(last = true, value_name = "ARG")]
        args: Vec<String>,

        /// Refuse to install or update an environment.
        #[arg(long)]
        no_install: bool,

        /// Print the planned action without changing files or running code.
        #[arg(long)]
        dry_run: bool,
    },

    /// Prepare dependencies without running code; preserve an existing project lock.
    Sync {
        /// Read inline metadata from this script or document.
        #[arg(long, value_name = "FILE")]
        script: Option<PathBuf>,

        /// Print the planned action without changing files.
        #[arg(long)]
        dry_run: bool,
    },

    /// Show the effective dependencies for a project or script.
    Deps {
        /// Read inline metadata from this script or document.
        #[arg(long, value_name = "FILE")]
        script: Option<PathBuf>,
    },

    /// Export dependency information to common ecosystem files.
    #[command(group(
        ArgGroup::new("format")
            .required(true)
            .multiple(false)
            .args(["requirements", "renv", "dockerfile"])
    ))]
    Export {
        /// Write Python indexes and dependencies to requirements.txt.
        #[arg(long)]
        requirements: bool,

        /// Write a conservative renv dependency helper.
        #[arg(long)]
        renv: bool,

        /// Write Dockerfile and extend .dockerignore with safety exclusions.
        #[arg(long)]
        dockerfile: bool,
    },

    /// Inspect configured tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },

    /// Manage Dual environment support.
    #[command(hide = true)]
    Engine {
        #[command(subcommand)]
        command: EngineCommand,
    },

    /// Manage the dual lockfile.
    Lock {
        #[command(subcommand)]
        command: LockCommand,
    },

    /// Open an interactive shell inside the project environment.
    Shell,

    /// Diagnose system tools and the current Dual project.
    Doctor,

    /// Remove files and environments generated by dual.
    Clean {
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// List configured tasks.
    List,

    /// Suggest common tasks from project files.
    Suggest,
}

#[derive(Debug, Subcommand)]
pub enum EngineCommand {
    /// Update Dual environment support.
    Update,

    /// Remove Dual environment support.
    Uninstall,
}

#[derive(Debug, Subcommand)]
pub enum LockCommand {
    /// Rewrite dual.lock using the current lockfile format.
    Migrate,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Language {
    R,
    #[value(alias = "python")]
    Py,
}

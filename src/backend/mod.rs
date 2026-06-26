mod engine;

use std::path::PathBuf;

use anyhow::Result;

use crate::config::Config;

pub use engine::{generate_manifest, EnvironmentBackend};

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BackendReport {
    pub available: bool,
    pub environment_present: bool,
    pub lock_present: bool,
    pub r_available: Option<bool>,
    pub python_available: Option<bool>,
    pub missing_r_packages: Vec<String>,
    pub missing_python_packages: Vec<String>,
    pub bridge: Option<BridgeReport>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BridgeReport {
    pub reticulate_installed: bool,
    pub uses_project_python: bool,
}

pub trait Backend {
    fn is_available(&self) -> bool;
    fn ensure_available(&self) -> Result<()>;
    fn update_engine(&self) -> Result<()>;
    fn uninstall_engine(&self) -> Result<bool>;
    fn migrate_lock(&self) -> Result<bool>;
    fn environment_exists(&self) -> bool;
    fn verify_manifest(&self, config: &Config) -> Result<()>;
    fn init_or_update(&self, config: &Config, refresh: bool) -> Result<()>;
    fn validate(&self, config: &Config) -> Result<()>;
    fn run(&self, config: &Config, task: &str) -> Result<()>;
    fn shell(&self, config: &Config) -> Result<()>;
    fn clean(&self) -> Result<Vec<PathBuf>>;
    fn doctor(&self, config: &Config) -> Result<BackendReport>;
}

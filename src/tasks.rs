use std::path::Path;

use anyhow::Result;

use crate::backend::Backend;
use crate::config::Config;
use crate::errors::DualError;

pub fn lookup<'a>(config: &'a Config, name: &str) -> Result<&'a str> {
    config.tasks.get(name).map(String::as_str).ok_or_else(|| {
        let available = if config.tasks.is_empty() {
            ". No tasks are configured in [tasks].".to_owned()
        } else {
            format!(
                ". Available tasks: {}",
                config.tasks.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        };
        DualError::MissingTask {
            name: name.to_owned(),
            available,
        }
        .into()
    })
}

pub fn run_task(root: &Path, backend: &impl Backend, name: &str) -> Result<()> {
    let config = Config::load(root)?;
    lookup(&config, name)?;

    if !backend.environment_exists() {
        anyhow::bail!("The project environment has not been created. Run `dual up` first.");
    }
    backend.ensure_available()?;

    println!("Running task `{name}`...");
    backend.run(name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{LanguageConfig, ProjectConfig};

    use super::*;

    fn config() -> Config {
        Config {
            project: ProjectConfig {
                name: "test".into(),
            },
            r: LanguageConfig {
                version: "4.5".into(),
                packages: vec![],
            },
            python: LanguageConfig {
                version: "3.12".into(),
                packages: vec![],
            },
            tasks: BTreeMap::from([("analysis".into(), "Rscript scripts/analysis.R".into())]),
        }
    }

    #[test]
    fn task_lookup_works() {
        assert_eq!(
            lookup(&config(), "analysis").unwrap(),
            "Rscript scripts/analysis.R"
        );
    }

    #[test]
    fn missing_task_lists_available_tasks() {
        let error = lookup(&config(), "missing").unwrap_err().to_string();
        assert!(error.contains("Available tasks: analysis"));
    }
}

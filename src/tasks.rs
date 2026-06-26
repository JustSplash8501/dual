use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::backend::Backend;
use crate::config::{Config, TaskConfig};
use crate::errors::DualError;
use crate::security;

pub fn lookup<'a>(config: &'a Config, name: &str) -> Result<&'a TaskConfig> {
    config.tasks.get(name).ok_or_else(|| {
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

pub fn run_task(
    root: &Path,
    backend: &impl Backend,
    name: &str,
    trust_project: bool,
) -> Result<()> {
    let config = Config::load(root)?;
    let task_order = resolve_task_order(&config, name)?;

    if !backend.environment_exists() {
        anyhow::bail!("The project environment has not been created. Run `dual up` first.");
    }
    let trust = security::ensure_project_trusted(root, trust_project)?;
    backend.ensure_available()?;
    backend.verify_manifest(&config)?;
    security::verify_project_unchanged(root, &trust)?;

    for task in &task_order {
        println!("Running task `{task}`...");
        backend.run(&config, task)?;
    }
    security::verify_project_unchanged(root, &trust)
}

pub fn list_tasks(root: &Path, json: bool) -> Result<()> {
    let config = Config::load(root)?;
    if json {
        let tasks = config
            .tasks
            .iter()
            .map(|(name, task)| TaskReport {
                name: name.clone(),
                command: task.command().to_owned(),
                deps: task.deps().to_vec(),
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&tasks)?);
    } else if config.tasks.is_empty() {
        println!("No tasks are configured.");
    } else {
        for (name, task) in config.tasks {
            if task.deps().is_empty() {
                println!("{name}\t{}", task.command());
            } else {
                println!(
                    "{name}\t{}\tdeps: {}",
                    task.command(),
                    task.deps().join(", ")
                );
            }
        }
    }
    Ok(())
}

fn resolve_task_order(config: &Config, name: &str) -> Result<Vec<String>> {
    let mut ordered = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    visit_task(config, name, &mut visiting, &mut visited, &mut ordered)?;
    Ok(ordered)
}

fn visit_task(
    config: &Config,
    name: &str,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<String>,
) -> Result<()> {
    if visited.contains(name) {
        return Ok(());
    }
    let task = lookup(config, name)?;
    if !visiting.insert(name.to_owned()) {
        anyhow::bail!("task dependency cycle includes `{name}`");
    }
    for dependency in task.deps() {
        visit_task(config, dependency, visiting, visited, ordered)?;
    }
    visiting.remove(name);
    visited.insert(name.to_owned());
    ordered.push(name.to_owned());
    Ok(())
}

#[derive(Serialize)]
struct TaskReport {
    name: String,
    command: String,
    deps: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{ProjectConfig, PythonConfig, QuartoConfig, RConfig};

    use super::*;

    fn config() -> Config {
        Config {
            project: ProjectConfig {
                name: "test".into(),
            },
            r: RConfig {
                enabled: true,
                version: "4.5".into(),
                packages: vec![],
            },
            python: PythonConfig {
                enabled: true,
                version: "3.12".into(),
                packages: vec![],
                index: vec![],
            },
            quarto: QuartoConfig::default(),
            tasks: BTreeMap::from([(
                "analysis".into(),
                TaskConfig::simple("Rscript scripts/analysis.R"),
            )]),
        }
    }

    #[test]
    fn task_lookup_works() {
        assert_eq!(
            lookup(&config(), "analysis").unwrap().command(),
            "Rscript scripts/analysis.R"
        );
    }

    #[test]
    fn missing_task_lists_available_tasks() {
        let error = lookup(&config(), "missing").unwrap_err().to_string();
        assert!(error.contains("Available tasks: analysis"));
    }
}

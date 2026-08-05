use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Serialize;

use crate::security;

const TEST_INCLUDES: &[&str] = &[
    "tests/**/*.py",
    "test/**/*.py",
    "**/test_*.py",
    "**/*_test.py",
    "tests/testthat/**/*.R",
    "tests/testthat/test-*.R",
];
const DEFAULT_EXCLUDES: &[&str] = &[
    ".git/**",
    ".dual/**",
    "target/**",
    "results/**",
    "__pycache__/**",
];
const DEFAULT_LIMITS: DiscoveryLimits = DiscoveryLimits {
    max_entries: 100_000,
    max_depth: 64,
};

#[derive(Clone, Copy, Debug)]
struct DiscoveryLimits {
    max_entries: usize,
    max_depth: usize,
}

#[derive(Default)]
struct WalkState {
    entries: usize,
}

struct WalkContext<'a> {
    root: &'a Path,
    includes: &'a GlobSet,
    excludes: &'a GlobSet,
    limits: DiscoveryLimits,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct TestDiscovery {
    pub python: Vec<PathBuf>,
    pub r: Vec<PathBuf>,
}

impl TestDiscovery {
    pub fn has_python(&self) -> bool {
        !self.python.is_empty()
    }

    pub fn has_r(&self) -> bool {
        !self.r.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        !self.has_python() && !self.has_r()
    }
}

pub fn discover_tests(root: &Path) -> Result<TestDiscovery> {
    discover_tests_with_limits(root, DEFAULT_LIMITS)
}

fn discover_tests_with_limits(root: &Path, limits: DiscoveryLimits) -> Result<TestDiscovery> {
    let includes = globset(TEST_INCLUDES)?;
    let excludes = globset(DEFAULT_EXCLUDES)?;
    let context = WalkContext {
        root,
        includes: &includes,
        excludes: &excludes,
        limits,
    };
    let mut discovery = TestDiscovery::default();
    let mut state = WalkState::default();
    walk_project(&context, root, &mut discovery, &mut state, 0)?;
    discovery.python.sort();
    discovery.r.sort();
    Ok(discovery)
}

fn walk_project(
    context: &WalkContext<'_>,
    directory: &Path,
    discovery: &mut TestDiscovery,
    state: &mut WalkState,
    depth: usize,
) -> Result<()> {
    if depth > context.limits.max_depth {
        anyhow::bail!(
            "project test discovery exceeded the {max_depth}-directory depth safety limit",
            max_depth = context.limits.max_depth
        );
    }
    for entry in fs::read_dir(directory)
        .with_context(|| format!("could not read {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        security::reject_symlink(&path, "project discovery path")?;
        let relative = path.strip_prefix(context.root).unwrap_or(&path);
        if context.excludes.is_match(relative) {
            continue;
        }
        state.entries += 1;
        if state.entries > context.limits.max_entries {
            anyhow::bail!(
                "project test discovery exceeded the {max_entries}-entry safety limit",
                max_entries = context.limits.max_entries
            );
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk_project(context, &path, discovery, state, depth + 1)?;
        } else if file_type.is_file() && context.includes.is_match(relative) {
            if relative
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
            {
                discovery.python.push(relative.to_path_buf());
            } else if relative
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("r"))
            {
                discovery.r.push(relative.to_path_buf());
            }
        }
    }
    Ok(())
}

fn globset(patterns: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .with_context(|| format!("invalid project glob pattern: {pattern}"))?,
        );
    }
    builder
        .build()
        .context("could not build project glob matcher")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_python_and_r_tests_with_default_exclusions() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("tests/testthat")).unwrap();
        fs::create_dir_all(directory.path().join("target/tests")).unwrap();
        fs::write(directory.path().join("tests/test_analysis.py"), "").unwrap();
        fs::write(directory.path().join("tests/testthat/test-analysis.R"), "").unwrap();
        fs::write(directory.path().join("target/tests/test_ignored.py"), "").unwrap();

        let discovery = discover_tests(directory.path()).unwrap();
        assert_eq!(
            discovery.python,
            vec![PathBuf::from("tests/test_analysis.py")]
        );
        assert_eq!(
            discovery.r,
            vec![PathBuf::from("tests/testthat/test-analysis.R")]
        );
    }

    #[test]
    fn refuses_to_walk_unbounded_project_trees() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("one/two/three")).unwrap();

        let error = discover_tests_with_limits(
            directory.path(),
            DiscoveryLimits {
                max_entries: 10,
                max_depth: 1,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("depth safety limit"));
    }
}

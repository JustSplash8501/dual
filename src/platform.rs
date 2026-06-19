use std::path::{Path, PathBuf};

pub fn managed_paths(root: &Path) -> Vec<PathBuf> {
    vec![root.join(".dual")]
}

pub fn referenced_script(command: &str) -> Option<PathBuf> {
    command
        .split_whitespace()
        .map(|part| part.trim_matches(|character| character == '"' || character == '\''))
        .find(|part| {
            let lower = part.to_ascii_lowercase();
            lower.ends_with(".r")
                || lower.ends_with(".py")
                || lower.ends_with(".qmd")
                || lower.ends_with(".rmd")
        })
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_paths_stay_inside_project() {
        let root = Path::new("project");
        let paths = managed_paths(root);
        assert!(paths.iter().all(|path| path.starts_with(root)));
        assert!(!paths.contains(&root.join("dual.lock")));
    }

    #[test]
    fn extracts_referenced_script() {
        assert_eq!(
            referenced_script("Rscript scripts/analysis.R"),
            Some(PathBuf::from("scripts/analysis.R"))
        );
        assert_eq!(
            referenced_script("python \"scripts/model.py\" --fast"),
            Some(PathBuf::from("scripts/model.py"))
        );
    }
}

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
pub const MAX_LOCK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PROJECT_FILES: usize = 100_000;
const MAX_PROJECT_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ProjectTrust {
    execution_fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct ProjectSnapshot {
    execution_fingerprint: String,
    excluded: Vec<PathBuf>,
}

pub fn default_dual_home() -> PathBuf {
    if let Some(path) = env::var_os("DUAL_HOME") {
        return PathBuf::from(path);
    }

    #[cfg(windows)]
    {
        env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("dual")
    }

    #[cfg(not(windows))]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join(".dual")
    }
}

pub fn read_text_file(path: &Path, limit: u64, label: &str) -> Result<String> {
    let bytes = read_file(path, limit, label)?;
    String::from_utf8(bytes).with_context(|| format!("{label} is not valid UTF-8"))
}

pub fn read_file(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>> {
    reject_symlink(path, label)?;
    let metadata =
        fs::metadata(path).with_context(|| format!("could not inspect {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("{label} is not a regular file: {}", path.display());
    }
    if metadata.len() > limit {
        anyhow::bail!("{label} exceeds the {limit}-byte safety limit");
    }

    let file =
        open_read_no_follow(path).with_context(|| format!("could not read {}", path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len().min(limit) as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        anyhow::bail!("{label} exceeds the {limit}-byte safety limit");
    }
    Ok(bytes)
}

pub fn write_file_atomic(path: &Path, contents: &[u8], label: &str) -> Result<()> {
    reject_symlink_if_present(path, label)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{label} has no parent directory"))?;
    let temporary = parent.join(format!(
        ".dual-write-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("could not create temporary {label}"))?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);

    #[cfg(windows)]
    if path.exists() {
        reject_symlink(path, label)?;
        fs::remove_file(path)?;
    }

    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("could not update {}", path.display()));
    }
    #[cfg(unix)]
    {
        let directory = fs::File::open(parent)
            .with_context(|| format!("could not open directory for {}", path.display()))?;
        directory
            .sync_all()
            .with_context(|| format!("could not sync directory for {}", path.display()))?;
    }
    Ok(())
}

pub fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("{label} must not be a symbolic link: {}", path.display());
    }
    Ok(())
}

pub fn reject_symlink_if_present(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            anyhow::bail!("{label} must not be a symbolic link: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", path.display())),
    }
}

pub fn ensure_managed_path(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("managed path escaped the project: {}", path.display()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => current.push(part),
            _ => anyhow::bail!("managed path is invalid: {}", path.display()),
        }
        reject_symlink_if_present(&current, "managed project path")?;
    }
    Ok(())
}

pub fn contains_control_characters(value: &str) -> bool {
    value.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
    })
}

pub fn ensure_project_trusted(root: &Path, authorize: bool) -> Result<ProjectTrust> {
    let fingerprints = project_fingerprints(root)?;
    let trusted = read_trust_record(&fingerprints.trust_path).is_ok_and(|record| {
        constant_time_eq(record.trim().as_bytes(), fingerprints.trust.as_bytes())
    });
    let authorized_by_environment = env::var("DUAL_TRUST_PROJECT")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));

    if trusted {
        return Ok(ProjectTrust {
            execution_fingerprint: fingerprints.execution,
        });
    }
    if authorize || authorized_by_environment {
        write_trust_record(&fingerprints.trust_path, &fingerprints.trust)?;
        return Ok(ProjectTrust {
            execution_fingerprint: fingerprints.execution,
        });
    }

    anyhow::bail!(
        "This project is not trusted, or project files changed. Review the project, then rerun \
         with `--trust-project`. Project configuration, package installation, lockfiles, and \
         tasks can execute code."
    )
}

pub fn project_is_trusted(root: &Path) -> Result<bool> {
    let fingerprint = project_fingerprint(root, true)?;
    let trust_path = trust_path(root)?;
    Ok(read_trust_record(&trust_path)
        .is_ok_and(|record| constant_time_eq(record.trim().as_bytes(), fingerprint.as_bytes())))
}

pub fn refresh_project_trust(root: &Path) -> Result<()> {
    let fingerprint = project_fingerprint(root, true)?;
    write_trust_record(&trust_path(root)?, &fingerprint)
}

pub fn verify_project_unchanged(root: &Path, trust: &ProjectTrust) -> Result<()> {
    let current = project_fingerprint(root, false)?;
    if !constant_time_eq(current.as_bytes(), trust.execution_fingerprint.as_bytes()) {
        anyhow::bail!(
            "Project files changed while code was executing. The changes were not trusted; \
             review them and rerun with `--trust-project`."
        );
    }
    Ok(())
}

pub fn snapshot_project_excluding(root: &Path, excluded: &[PathBuf]) -> Result<ProjectSnapshot> {
    let excluded = excluded
        .iter()
        .map(|path| path.strip_prefix(root).unwrap_or(path).to_path_buf())
        .collect::<Vec<_>>();
    for path in &excluded {
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            anyhow::bail!(
                "excluded project output path is invalid: {}",
                path.display()
            );
        }
    }
    Ok(ProjectSnapshot {
        execution_fingerprint: project_fingerprint_excluding(root, false, &excluded)?,
        excluded,
    })
}

pub fn verify_project_snapshot(root: &Path, snapshot: &ProjectSnapshot) -> Result<()> {
    let current = project_fingerprint_excluding(root, false, &snapshot.excluded)?;
    if !constant_time_eq(
        current.as_bytes(),
        snapshot.execution_fingerprint.as_bytes(),
    ) {
        anyhow::bail!(
            "Project source files changed while code was executing. The changes were not \
             trusted; review them and rerun with `--trust-project`."
        );
    }
    Ok(())
}

fn project_fingerprint(root: &Path, include_lock: bool) -> Result<String> {
    project_fingerprint_excluding(root, include_lock, &[])
}

struct ProjectFingerprints {
    trust: String,
    execution: String,
    trust_path: PathBuf,
}

fn project_fingerprints(root: &Path) -> Result<ProjectFingerprints> {
    let canonical = fs::canonicalize(root)
        .with_context(|| format!("could not canonicalize project root {}", root.display()))?;
    let dual_home = excluded_dual_home_relative(&canonical);
    let cache = excluded_cache_relative(&canonical);
    let mut trust = ProjectHasher::new(&canonical);
    let mut execution = ProjectHasher::without_lock(&canonical);
    let mut files = 0;
    let mut bytes = 0;
    hash_project_directory(
        &HashContext {
            root,
            include_lock: true,
            excluded: &[],
            excluded_dual_home: dual_home.as_deref(),
            excluded_cache: cache.as_deref(),
        },
        root,
        &mut [&mut trust, &mut execution],
        &mut files,
        &mut bytes,
    )?;
    Ok(ProjectFingerprints {
        trust: trust.finish(),
        execution: execution.finish(),
        trust_path: trust_path_for_canonical_root(&canonical),
    })
}

fn project_fingerprint_excluding(
    root: &Path,
    include_lock: bool,
    excluded: &[PathBuf],
) -> Result<String> {
    let canonical = fs::canonicalize(root)
        .with_context(|| format!("could not canonicalize project root {}", root.display()))?;
    let dual_home = excluded_dual_home_relative(&canonical);
    let cache = excluded_cache_relative(&canonical);
    let mut hasher = ProjectHasher::new(&canonical);
    let mut files = 0;
    let mut bytes = 0;
    hash_project_directory(
        &HashContext {
            root,
            include_lock,
            excluded,
            excluded_dual_home: dual_home.as_deref(),
            excluded_cache: cache.as_deref(),
        },
        root,
        &mut [&mut hasher],
        &mut files,
        &mut bytes,
    )?;
    Ok(hasher.finish())
}

struct HashContext<'a> {
    root: &'a Path,
    include_lock: bool,
    excluded: &'a [PathBuf],
    excluded_dual_home: Option<&'a Path>,
    excluded_cache: Option<&'a Path>,
}

struct ProjectHasher {
    hasher: Sha256,
    include_lock: bool,
}

impl ProjectHasher {
    fn new(root: &Path) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"dual-project-trust-v2\0");
        hash_path(&mut hasher, root);
        hasher.update([0]);
        Self {
            hasher,
            include_lock: true,
        }
    }

    fn without_lock(root: &Path) -> Self {
        let mut hasher = Self::new(root);
        hasher.include_lock = false;
        hasher
    }

    fn hashes_file(&self, relative: &Path) -> bool {
        self.include_lock || relative != Path::new("dual.lock")
    }

    fn start_file(&mut self, relative: &Path, size: u64) {
        self.hasher.update(b"file\0");
        hash_path(&mut self.hasher, relative);
        self.hasher.update([0]);
        self.hasher.update(size.to_le_bytes());
    }

    fn update_file_contents(&mut self, contents: &[u8]) {
        self.hasher.update(contents);
    }

    fn finish(self) -> String {
        format!("{:x}", self.hasher.finalize())
    }
}

fn hash_project_directory(
    context: &HashContext<'_>,
    directory: &Path,
    hashers: &mut [&mut ProjectHasher],
    files: &mut usize,
    bytes: &mut u64,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| {
            format!(
                "could not inspect project directory {}",
                directory.display()
            )
        })?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(context.root)
            .with_context(|| format!("project path escaped its root: {}", path.display()))?;
        let first = relative.components().next();
        if context
            .excluded
            .iter()
            .any(|excluded| relative == excluded || relative.starts_with(excluded))
        {
            continue;
        }
        if matches!(
            first,
            Some(Component::Normal(name)) if name == ".git" || name == ".dual" || name == "results"
        ) || (!context.include_lock && relative == Path::new("dual.lock"))
            || context
                .excluded_dual_home
                .is_some_and(|dual_home| relative.starts_with(dual_home))
            || context
                .excluded_cache
                .is_some_and(|cache| relative.starts_with(cache))
        {
            continue;
        }

        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("could not inspect project path {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            anyhow::bail!(
                "trusted projects cannot contain symbolic links: {}",
                relative.display()
            );
        }
        if metadata.is_dir() {
            hash_project_directory(context, &path, hashers, files, bytes)?;
            continue;
        }
        if !metadata.is_file() {
            anyhow::bail!(
                "trusted projects cannot contain special files: {}",
                relative.display()
            );
        }

        *files += 1;
        *bytes = bytes
            .checked_add(metadata.len())
            .ok_or_else(|| anyhow::anyhow!("project size overflow"))?;
        if *files > MAX_PROJECT_FILES || *bytes > MAX_PROJECT_BYTES {
            anyhow::bail!(
                "project exceeds trust limits of {MAX_PROJECT_FILES} files or \
                 {MAX_PROJECT_BYTES} bytes"
            );
        }

        let mut file = open_read_no_follow(&path)
            .with_context(|| format!("could not read project file {}", path.display()))?;
        for hasher in hashers.iter_mut() {
            if hasher.hashes_file(relative) {
                hasher.start_file(relative, metadata.len());
            }
        }
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            for hasher in hashers.iter_mut() {
                if hasher.hashes_file(relative) {
                    hasher.update_file_contents(&buffer[..read]);
                }
            }
        }
        if file.metadata()?.len() != metadata.len() {
            anyhow::bail!(
                "project file changed while it was being reviewed: {}",
                relative.display()
            );
        }
    }
    Ok(())
}

fn excluded_dual_home_relative(root: &Path) -> Option<PathBuf> {
    let home = normalize_identity_path(&default_dual_home());
    home.is_absolute()
        .then(|| home.strip_prefix(root).ok().map(Path::to_path_buf))
        .flatten()
        .filter(|path| !path.as_os_str().is_empty())
}

fn excluded_cache_relative(root: &Path) -> Option<PathBuf> {
    let cache = normalize_identity_path(&crate::cache::owned_cache_dir()?);
    cache
        .is_absolute()
        .then(|| cache.strip_prefix(root).ok().map(Path::to_path_buf))
        .flatten()
        .filter(|path| !path.as_os_str().is_empty())
}

fn normalize_identity_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let mut current = path;
    let mut missing = Vec::new();
    while let Some(name) = current.file_name() {
        missing.push(name.to_owned());
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
        if let Ok(mut canonical) = fs::canonicalize(current) {
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return normalize_lexical_path(&canonical);
        }
    }
    normalize_lexical_path(path)
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn trust_path(root: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(root)
        .with_context(|| format!("could not canonicalize project root {}", root.display()))?;
    Ok(trust_path_for_canonical_root(&canonical))
}

fn trust_path_for_canonical_root(canonical: &Path) -> PathBuf {
    let mut hasher = Sha256::new();
    hash_path(&mut hasher, canonical);
    let key = hasher.finalize();
    default_dual_home()
        .join("trust")
        .join(format!("{key:x}.sha256"))
}

fn hash_path(hasher: &mut Sha256, path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for code_unit in path.as_os_str().encode_wide() {
            hasher.update(code_unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
    hasher.update(path.as_os_str().to_string_lossy().as_bytes());
}

fn read_trust_record(path: &Path) -> Result<String> {
    read_text_file(path, 256, "project trust record")
}

fn write_trust_record(path: &Path, fingerprint: &str) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("project trust path has no parent"))?;
    let home = default_dual_home();
    create_private_directory(&home, "dual data directory")?;
    create_private_directory(directory, "project trust directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    write_file_atomic(
        path,
        format!("{fingerprint}\n").as_bytes(),
        "project trust record",
    )
}

pub fn create_private_directory(path: &Path, label: &str) -> Result<()> {
    reject_symlink_if_present(path, label)?;
    if !path.exists() {
        fs::create_dir(path).with_context(|| format!("could not create {label}"))?;
    }
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        anyhow::bail!("{label} is not a directory: {}", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn open_read_no_follow(path: &Path) -> Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    #[test]
    fn lexical_normalization_resolves_parent_components() {
        assert_eq!(
            normalize_lexical_path(Path::new("/project/scripts/../dual-home")),
            PathBuf::from("/project/dual-home")
        );
    }

    #[test]
    fn native_non_utf8_paths_do_not_collide_in_trust_hashes() {
        let mut first = Sha256::new();
        hash_path(&mut first, Path::new(OsStr::from_bytes(b"script-\x80")));
        let mut second = Sha256::new();
        hash_path(&mut second, Path::new(OsStr::from_bytes(b"script-\x81")));

        assert_ne!(first.finalize(), second.finalize());
    }

    #[test]
    fn scoped_snapshot_allows_outputs_but_detects_source_changes() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("report.qmd"), "source").unwrap();
        let snapshot =
            snapshot_project_excluding(directory.path(), &[directory.path().join("report.html")])
                .unwrap();
        fs::write(directory.path().join("report.html"), "generated").unwrap();
        verify_project_snapshot(directory.path(), &snapshot).unwrap();

        fs::write(directory.path().join("report.qmd"), "changed").unwrap();
        assert!(verify_project_snapshot(directory.path(), &snapshot).is_err());
    }

    #[test]
    fn combined_project_fingerprints_match_single_fingerprints() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("dual.toml"), "config").unwrap();
        fs::write(directory.path().join("dual.lock"), "lock").unwrap();

        let combined = project_fingerprints(directory.path()).unwrap();

        assert_eq!(
            combined.trust,
            project_fingerprint(directory.path(), true).unwrap()
        );
        assert_eq!(
            combined.execution,
            project_fingerprint(directory.path(), false).unwrap()
        );
        assert_eq!(combined.trust_path, trust_path(directory.path()).unwrap());
        assert_ne!(combined.trust, combined.execution);
    }
}

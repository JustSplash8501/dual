use std::env;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::security;

const CACHE_LAYOUT_VERSION: u32 = 1;
const CACHE_MARKER: &str = ".dual-cache";
const CACHE_MARKER_CONTENTS: &str = "dual-cache-v1\n";
const CACHE_LOCK: &str = ".lock";
const CACHE_ACTIVE_ENV: &str = "DUAL_CACHE_ACTIVE";

#[derive(Clone, Debug, Serialize)]
pub struct CacheReport {
    pub directory: PathBuf,
    pub layout_version: u32,
    pub exists: bool,
    pub files: u64,
    pub bytes: u64,
    pub buckets: Vec<CacheBucketReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CacheBucketReport {
    pub name: &'static str,
    pub directory: PathBuf,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CacheRemovalReport {
    pub files: u64,
    pub bytes: u64,
    pub directories: u64,
}

pub struct CacheGuard {
    file: File,
}

impl Drop for CacheGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

pub fn cache_dir() -> Result<PathBuf> {
    let configured = env::var_os("DUAL_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_cache_dir);
    absolute_path(configured)
}

pub fn owned_cache_dir() -> Option<PathBuf> {
    let root = cache_dir().ok()?;
    cache_marker_is_valid(&root).then_some(root)
}

pub fn environment_cache_dir() -> Result<PathBuf> {
    Ok(current_layout_dir()?.join("environment"))
}

pub fn r_package_cache_dir() -> Result<PathBuf> {
    Ok(current_layout_dir()?.join("r").join("packages"))
}

pub fn r_metadata_cache_dir() -> Result<PathBuf> {
    Ok(current_layout_dir()?.join("r").join("metadata"))
}

pub fn prepare() -> Result<()> {
    let root = cache_dir()?;
    prepare_root(&root)?;
    create_private_tree(&current_layout_dir()?)
}

pub fn lock_shared() -> Result<CacheGuard> {
    let file = open_lock_file()?;
    fs2::FileExt::lock_shared(&file).context("could not lock the Dual package cache")?;
    Ok(CacheGuard { file })
}

fn lock_exclusive() -> Result<CacheGuard> {
    let file = open_lock_file()?;
    fs2::FileExt::lock_exclusive(&file).context("could not lock the Dual package cache")?;
    Ok(CacheGuard { file })
}

pub fn inspect() -> Result<CacheReport> {
    let root = cache_dir()?;
    if !root.exists() {
        return Ok(CacheReport {
            directory: root,
            layout_version: CACHE_LAYOUT_VERSION,
            exists: false,
            files: 0,
            bytes: 0,
            buckets: empty_bucket_reports()?,
        });
    }
    validate_root(&root)?;
    let _guard = lock_shared()?;
    let measured = usage(&current_layout_dir()?)?;
    let buckets = bucket_paths()?
        .into_iter()
        .map(|(name, directory)| {
            let measured = usage(&directory)?;
            Ok(CacheBucketReport {
                name,
                directory,
                files: measured.files,
                bytes: measured.bytes,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CacheReport {
        directory: root,
        layout_version: CACHE_LAYOUT_VERSION,
        exists: true,
        files: measured.files,
        bytes: measured.bytes,
        buckets,
    })
}

pub fn clean() -> Result<CacheRemovalReport> {
    ensure_maintenance_allowed()?;
    prepare()?;
    let _guard = lock_exclusive()?;
    let root = cache_dir()?;
    let mut removed = CacheRemovalReport::default();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry.file_name();
        if is_layout_directory(&name.to_string_lossy()) {
            add_removal(&mut removed, remove_tree(&entry.path())?);
        }
    }
    Ok(removed)
}

pub fn prune() -> Result<CacheRemovalReport> {
    ensure_maintenance_allowed()?;
    prepare()?;
    let _guard = lock_exclusive()?;
    let root = cache_dir()?;
    let current = format!("v{CACHE_LAYOUT_VERSION}");
    let mut removed = CacheRemovalReport::default();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_layout_directory(&name) && name != current {
            add_removal(&mut removed, remove_tree(&entry.path())?);
        }
    }
    Ok(removed)
}

fn default_cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("dual")
            .join("cache")
    }

    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("Library")
            .join("Caches")
            .join("dual")
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(env::temp_dir)
            .join("dual")
    }
}

fn ensure_maintenance_allowed() -> Result<()> {
    if env::var(CACHE_ACTIVE_ENV).is_ok_and(|value| value == "1") {
        anyhow::bail!(
            "cache maintenance cannot run inside an active Dual task or shell; run it after exiting"
        );
    }
    Ok(())
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn current_layout_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join(format!("v{CACHE_LAYOUT_VERSION}")))
}

fn bucket_paths() -> Result<Vec<(&'static str, PathBuf)>> {
    let current = current_layout_dir()?;
    Ok(vec![
        ("environment", current.join("environment")),
        ("r-packages", current.join("r").join("packages")),
        ("r-metadata", current.join("r").join("metadata")),
    ])
}

fn empty_bucket_reports() -> Result<Vec<CacheBucketReport>> {
    Ok(bucket_paths()?
        .into_iter()
        .map(|(name, directory)| CacheBucketReport {
            name,
            directory,
            files: 0,
            bytes: 0,
        })
        .collect())
}

fn prepare_root(root: &Path) -> Result<()> {
    if root.exists() {
        validate_root(root)?;
        return Ok(());
    }
    create_private_tree(root)?;
    security::write_file_atomic(
        &root.join(CACHE_MARKER),
        CACHE_MARKER_CONTENTS.as_bytes(),
        "Dual cache marker",
    )
}

fn validate_root(root: &Path) -> Result<()> {
    security::reject_symlink(root, "Dual package cache directory")?;
    if !root.is_dir() {
        anyhow::bail!("Dual package cache is not a directory: {}", root.display());
    }
    let marker = root.join(CACHE_MARKER);
    if !marker.exists() {
        let empty = fs::read_dir(root)?.next().is_none();
        if empty {
            security::write_file_atomic(
                &marker,
                CACHE_MARKER_CONTENTS.as_bytes(),
                "Dual cache marker",
            )?;
            return Ok(());
        }
        anyhow::bail!(
            "refusing to use non-empty cache directory without a Dual marker: {}",
            root.display()
        );
    }
    if !cache_marker_is_valid(root) {
        anyhow::bail!("Dual cache marker is invalid: {}", marker.display());
    }
    Ok(())
}

fn cache_marker_is_valid(root: &Path) -> bool {
    let Ok(root_metadata) = fs::symlink_metadata(root) else {
        return false;
    };
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return false;
    }
    let marker = root.join(CACHE_MARKER);
    let Ok(marker_metadata) = fs::symlink_metadata(&marker) else {
        return false;
    };
    marker_metadata.is_file()
        && !marker_metadata.file_type().is_symlink()
        && fs::read_to_string(marker).is_ok_and(|contents| contents == CACHE_MARKER_CONTENTS)
}

fn create_private_tree(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("could not create Dual package cache at {}", path.display()))?;
    security::reject_symlink(path, "Dual package cache directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn open_lock_file() -> Result<File> {
    prepare()?;
    let path = cache_dir()?.join(CACHE_LOCK);
    security::reject_symlink_if_present(&path, "Dual package cache lock")?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .context("could not open the Dual package cache lock")
}

#[derive(Clone, Copy, Debug, Default)]
struct Usage {
    files: u64,
    bytes: u64,
    directories: u64,
}

fn usage(path: &Path) -> Result<Usage> {
    if !path.exists() {
        return Ok(Usage::default());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return Ok(Usage {
            files: 1,
            bytes: metadata.len(),
            directories: 0,
        });
    }
    let mut result = Usage {
        directories: 1,
        ..Usage::default()
    };
    for entry in fs::read_dir(path)? {
        let child = usage(&entry?.path())?;
        result.files += child.files;
        result.bytes += child.bytes;
        result.directories += child.directories;
    }
    Ok(result)
}

fn remove_tree(path: &Path) -> Result<CacheRemovalReport> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let mut removed = CacheRemovalReport {
            directories: 1,
            ..CacheRemovalReport::default()
        };
        for entry in fs::read_dir(path)? {
            add_removal(&mut removed, remove_tree(&entry?.path())?);
        }
        fs::remove_dir(path)?;
        Ok(removed)
    } else {
        fs::remove_file(path)?;
        Ok(CacheRemovalReport {
            files: 1,
            bytes: metadata.len(),
            directories: 0,
        })
    }
}

fn is_layout_directory(name: &str) -> bool {
    name.strip_prefix('v')
        .is_some_and(|version| !version.is_empty() && version.chars().all(|c| c.is_ascii_digit()))
}

fn add_removal(total: &mut CacheRemovalReport, addition: CacheRemovalReport) {
    total.files += addition.files;
    total.bytes += addition.bytes;
    total.directories += addition.directories;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_names_are_strict() {
        assert!(is_layout_directory("v1"));
        assert!(is_layout_directory("v42"));
        assert!(!is_layout_directory("v"));
        assert!(!is_layout_directory("version1"));
        assert!(!is_layout_directory("v1-backup"));
    }
}

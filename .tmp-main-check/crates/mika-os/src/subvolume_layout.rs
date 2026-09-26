use std::path::{Path, PathBuf};

use crate::btrfs;
use crate::error::MikaOsError;

/// Nested subvolume for logs (excluded from parent snapshots).
pub const LOGS_NESTED_SUBVOL: &str = "logs";

/// Nested subvolume for old copy-backups (excluded from parent snapshots).
pub const BACKUPS_NESTED_SUBVOL: &str = "data/_backups";

/// Directory for snapshots.
pub const SNAPSHOTS_DIR: &str = ".snapshots";

/// Marker file indicating time-machine is enabled.
pub const ENABLED_MARKER: &str = ".snapshots/.enabled";

/// Nested subvolumes that are excluded from parent snapshots.
pub const NESTED_SUBVOLUMES: &[&str] = &[LOGS_NESTED_SUBVOL, BACKUPS_NESTED_SUBVOL];

/// Status of the subvolume layout at `~/.mika/`.
#[derive(Debug, PartialEq, Eq)]
pub enum LayoutStatus {
    /// All subvolumes present and correct.
    Valid,
    /// Layout is partially set up (some nested subvols missing).
    Partial { missing: Vec<String> },
    /// Not a btrfs filesystem.
    NotBtrfs,
    /// Main path is not a subvolume.
    NotSubvolume,
}

/// Check if time-machine is enabled at the given home directory.
pub fn is_enabled(home: &Path) -> bool {
    home.join(ENABLED_MARKER).exists()
}

/// Validate the subvolume layout at `home` (~/.mika/).
pub fn validate_layout(home: &Path) -> Result<LayoutStatus, MikaOsError> {
    if !btrfs::is_btrfs(home)? {
        return Ok(LayoutStatus::NotBtrfs);
    }

    if !btrfs::is_subvolume(home)? {
        return Ok(LayoutStatus::NotSubvolume);
    }

    let mut missing = Vec::new();
    for subvol in NESTED_SUBVOLUMES {
        let subvol_path = home.join(subvol);
        if subvol_path.exists() {
            if !btrfs::is_subvolume(&subvol_path)? {
                missing.push(format!("{subvol} (exists but not a subvolume)"));
            }
        } else {
            missing.push(subvol.to_string());
        }
    }

    if missing.is_empty() {
        Ok(LayoutStatus::Valid)
    } else {
        Ok(LayoutStatus::Partial { missing })
    }
}

/// Create the full subvolume layout at `home`, including nested subvolumes.
/// Idempotent — skips subvolumes that already exist.
pub fn initialize_layout(home: &Path) -> Result<(), MikaOsError> {
    for subvol in NESTED_SUBVOLUMES {
        let subvol_path = home.join(subvol);
        if subvol_path.exists() {
            if btrfs::is_subvolume(&subvol_path)? {
                continue; // Already a subvolume
            }
            // Exists but not a subvolume — can't create over it
            return Err(MikaOsError::LayoutInvalid(format!(
                "{subvol} exists but is not a subvolume; remove it first"
            )));
        }

        // Ensure parent directory exists
        if let Some(parent) = subvol_path.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
        }

        btrfs::create_subvolume(&subvol_path)?;
    }

    Ok(())
}

/// Return the snapshots directory path.
pub fn snapshots_dir(home: &Path) -> PathBuf {
    home.join(SNAPSHOTS_DIR)
}

/// Return the enabled marker file path.
pub fn enabled_marker_path(home: &Path) -> PathBuf {
    home.join(ENABLED_MARKER)
}

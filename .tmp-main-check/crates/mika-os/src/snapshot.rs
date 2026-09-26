use std::fs;
use std::path::{Path, PathBuf};

use crate::btrfs;
use crate::error::MikaOsError;
use crate::subvolume_layout;

/// Label for a snapshot, encoding tenant, session, and timestamp.
#[derive(Debug, Clone)]
pub struct SnapshotLabel {
    pub tenant_id: String,
    pub session_id: String,
    pub timestamp: String, // ISO 8601
}

impl SnapshotLabel {
    /// Format as a directory name: `{tenant_id}_{session_id}_{timestamp}`.
    /// Timestamp colons are replaced with dots for filesystem safety
    /// (dashes are kept as-is since they appear in the date part).
    pub fn to_dir_name(&self) -> String {
        let safe_ts = self.timestamp.replace(':', ".");
        format!("{}_{}_{}", self.tenant_id, self.session_id, safe_ts)
    }

    /// Parse a directory name back into a label.
    pub fn from_dir_name(name: &str) -> Option<Self> {
        let parts: Vec<&str> = name.splitn(3, '_').collect();
        if parts.len() != 3 {
            return None;
        }
        Some(Self {
            tenant_id: parts[0].to_string(),
            session_id: parts[1].to_string(),
            timestamp: parts[2].replace('.', ":"),
        })
    }
}

/// Information about a stored snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub label: SnapshotLabel,
    pub path: PathBuf,
}

/// Result of a prune operation.
#[derive(Debug)]
pub struct PruneResult {
    pub deleted: usize,
    pub remaining: usize,
}

/// Create a read-only snapshot of `home` (~/.mika/).
///
/// Returns the path to the created snapshot.
pub fn create_snapshot(home: &Path, label: &SnapshotLabel) -> Result<PathBuf, MikaOsError> {
    if !subvolume_layout::is_enabled(home) {
        return Err(MikaOsError::NotEnabled);
    }

    let snap_dir = subvolume_layout::snapshots_dir(home);
    let snap_path = snap_dir.join(label.to_dir_name());

    if snap_path.exists() {
        // Idempotent — snapshot already exists
        return Ok(snap_path);
    }

    btrfs::snapshot_readonly(home, &snap_path)?;
    Ok(snap_path)
}

/// List all snapshots under `home/.snapshots/`.
pub fn list_snapshots(
    home: &Path,
    tenant_filter: Option<&str>,
) -> Result<Vec<SnapshotInfo>, MikaOsError> {
    let snap_dir = subvolume_layout::snapshots_dir(home);
    if !snap_dir.exists() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();
    for entry in fs::read_dir(&snap_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip the .enabled marker
        if name_str.starts_with('.') {
            continue;
        }

        if let Some(label) = SnapshotLabel::from_dir_name(&name_str) {
            if let Some(filter) = tenant_filter
                && label.tenant_id != filter
            {
                continue;
            }
            snapshots.push(SnapshotInfo {
                label,
                path: entry.path(),
            });
        }
    }

    // Sort by timestamp (newest first)
    snapshots.sort_by(|a, b| b.label.timestamp.cmp(&a.label.timestamp));
    Ok(snapshots)
}

/// Delete a single snapshot subvolume.
pub fn delete_snapshot(snap_path: &Path) -> Result<(), MikaOsError> {
    if !snap_path.exists() {
        return Err(MikaOsError::SnapshotNotFound(
            snap_path.display().to_string(),
        ));
    }
    btrfs::delete_subvolume(snap_path)
}

/// Prune snapshots, keeping the `keep` most recent.
/// Returns the number of deleted and remaining snapshots.
pub fn prune_snapshots(
    home: &Path,
    keep: usize,
    tenant_filter: Option<&str>,
) -> Result<PruneResult, MikaOsError> {
    let snapshots = list_snapshots(home, tenant_filter)?;

    if snapshots.len() <= keep {
        return Ok(PruneResult {
            deleted: 0,
            remaining: snapshots.len(),
        });
    }

    let to_delete = &snapshots[keep..];
    let mut deleted = 0;
    for snap in to_delete {
        match delete_snapshot(&snap.path) {
            Ok(()) => deleted += 1,
            Err(e) => {
                tracing::warn!(path = %snap.path.display(), error = %e, "failed to delete snapshot during prune");
            }
        }
    }

    Ok(PruneResult {
        deleted,
        remaining: snapshots.len() - deleted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_round_trip() {
        let label = SnapshotLabel {
            tenant_id: "mika".to_string(),
            session_id: "abc123".to_string(),
            timestamp: "2026-06-07T10:30:00Z".to_string(),
        };

        let dir_name = label.to_dir_name();
        assert_eq!(dir_name, "mika_abc123_2026-06-07T10.30.00Z");

        let parsed = SnapshotLabel::from_dir_name(&dir_name).unwrap();
        assert_eq!(parsed.tenant_id, "mika");
        assert_eq!(parsed.session_id, "abc123");
        assert_eq!(parsed.timestamp, "2026-06-07T10:30:00Z");
    }

    #[test]
    fn label_parse_invalid() {
        assert!(SnapshotLabel::from_dir_name("no-underscores").is_none());
        assert!(SnapshotLabel::from_dir_name("one_part").is_none());
    }

    #[test]
    fn label_preserves_dashes_in_timestamp() {
        let label = SnapshotLabel {
            tenant_id: "t1".to_string(),
            session_id: "s1".to_string(),
            timestamp: "2026-06-07T10:30:00Z".to_string(),
        };
        let dir = label.to_dir_name();
        // Colons are replaced with dots, dashes preserved
        assert!(!dir.contains(':'));
        assert!(dir.contains('-')); // Date dashes preserved
    }
}

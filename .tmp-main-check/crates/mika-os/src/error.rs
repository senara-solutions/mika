use thiserror::Error;

#[derive(Debug, Error)]
pub enum MikaOsError {
    #[error("btrfs is not available at path: {0}")]
    NotBtrfs(String),

    #[error("btrfs command failed: {0}")]
    BtrfsCommand(String),

    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("time-machine is not enabled (run `mika snapshot enable` first)")]
    NotEnabled,

    #[error("subvolume layout validation failed: {0}")]
    LayoutInvalid(String),

    #[error("redaction failed: {0}")]
    RedactionFailed(String),

    #[error("fork failed: {0}")]
    ForkFailed(String),

    #[error("rollback failed: {0}")]
    RollbackFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

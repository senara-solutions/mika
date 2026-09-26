use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

use crate::error::MikaOsError;

/// Check if the given path resides on a btrfs filesystem.
pub fn is_btrfs(path: &Path) -> Result<bool, MikaOsError> {
    // Use `btrfs filesystem df` which exits 0 only on btrfs.
    let output = Command::new("btrfs")
        .args(["filesystem", "df", path.to_str().unwrap_or(".")])
        .output();

    match output {
        Ok(o) => Ok(o.status.success()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // btrfs-progs not installed
            Ok(false)
        }
        Err(e) => Err(MikaOsError::Io(e)),
    }
}

/// Create a new btrfs subvolume at the given path.
pub fn create_subvolume(path: &Path) -> Result<(), MikaOsError> {
    run_btrfs_cmd(&["subvolume", "create", &path_str(path)?])
}

/// Delete a btrfs subvolume at the given path.
pub fn delete_subvolume(path: &Path) -> Result<(), MikaOsError> {
    run_btrfs_cmd(&["subvolume", "delete", &path_str(path)?])
}

/// Create a read-only snapshot of `src` at `dst`.
pub fn snapshot_readonly(src: &Path, dst: &Path) -> Result<(), MikaOsError> {
    run_btrfs_cmd(&[
        "subvolume",
        "snapshot",
        "-r",
        &path_str(src)?,
        &path_str(dst)?,
    ])
}

/// Create a writable snapshot of `src` at `dst`.
pub fn snapshot_writable(src: &Path, dst: &Path) -> Result<(), MikaOsError> {
    run_btrfs_cmd(&["subvolume", "snapshot", &path_str(src)?, &path_str(dst)?])
}

/// Check if a path is a btrfs subvolume.
pub fn is_subvolume(path: &Path) -> Result<bool, MikaOsError> {
    let output = Command::new("btrfs")
        .args(["subvolume", "show", &path_str(path)?])
        .output()?;
    Ok(output.status.success())
}

/// Stream a btrfs send of the given snapshot to the provided writer.
pub fn send_stream(snap: &Path, stdout: &mut impl Write) -> Result<(), MikaOsError> {
    let output = Command::new("btrfs")
        .args(["send", &path_str(snap)?])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MikaOsError::BtrfsCommand(format!(
            "btrfs send failed: {stderr}"
        )));
    }

    stdout.write_all(&output.stdout)?;
    Ok(())
}

/// Receive a btrfs send stream from the provided reader into `dest_dir`.
pub fn receive_stream(dest_dir: &Path, stdin: &mut impl Read) -> Result<(), MikaOsError> {
    use std::process::Stdio;

    let mut child = Command::new("btrfs")
        .args(["receive", &path_str(dest_dir)?])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(ref mut child_stdin) = child.stdin {
        let mut buf = Vec::new();
        stdin.read_to_end(&mut buf)?;
        child_stdin.write_all(&buf)?;
    }
    // Drop stdin to signal EOF
    drop(child.stdin.take());

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MikaOsError::BtrfsCommand(format!(
            "btrfs receive failed: {stderr}"
        )));
    }

    Ok(())
}

fn path_str(path: &Path) -> Result<String, MikaOsError> {
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| MikaOsError::BtrfsCommand("path contains invalid UTF-8".to_string()))
}

fn run_btrfs_cmd(args: &[&str]) -> Result<(), MikaOsError> {
    let output = Command::new("btrfs").args(args).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MikaOsError::BtrfsCommand(format!(
            "btrfs {} failed: {stderr}",
            args.join(" ")
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_str_handles_valid_utf8() {
        let p = Path::new("/home/mika/.mika");
        assert_eq!(path_str(p).unwrap(), "/home/mika/.mika");
    }
}

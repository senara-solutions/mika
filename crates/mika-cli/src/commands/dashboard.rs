use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use mika_common::home;

use crate::cli::DashboardCommand;

const DASHBOARD_URL: &str = "http://localhost:5173";

/// Run the dashboard subcommand.
pub async fn run(command: DashboardCommand) -> Result<()> {
    match command {
        DashboardCommand::Start => start().await,
        DashboardCommand::Stop => stop(),
        DashboardCommand::Status => status(),
        DashboardCommand::Open => open(),
    }
}

/// Resolve the PID file path: `~/.mika/dashboard.pid`.
pub fn pid_file_path() -> Result<PathBuf> {
    let home = home::resolve_home_dir()?;
    Ok(home.join("dashboard.pid"))
}

/// Check if a process with the given PID is alive.
/// Uses kill(1) with signal 0 for POSIX portability (works on both Linux and macOS).
fn is_process_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Send a signal to a process via kill(1).
fn send_signal(pid: u32, signal: i32) {
    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .output();
}

/// Read and validate the PID file. Returns `Some(pid)` if the process is alive.
pub fn read_pid() -> Option<u32> {
    let path = pid_file_path().ok()?;
    let content = fs::read_to_string(&path).ok()?;
    let pid: u32 = content.trim().parse().ok()?;
    if is_process_alive(pid) {
        Some(pid)
    } else {
        // Stale PID file — clean up
        let _ = fs::remove_file(&path);
        None
    }
}

/// Check if the dashboard dev server is running (used by TUI for status polling).
pub fn is_dashboard_running() -> bool {
    read_pid().is_some()
}

/// Walk up from the current directory to find the Mika project root.
pub fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let pkg = dir.join("package.json");
        if pkg.exists()
            && let Ok(content) = fs::read_to_string(&pkg)
            && content.contains("dev:dashboard")
        {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Start the dashboard dev server as a background process.
/// Returns `(pid, message)` on success.
///
/// Shared between CLI `start` and TUI `/dashboard start` handler.
pub fn start_dashboard_process() -> Result<(u32, String)> {
    // Check if already running
    if let Some(pid) = read_pid() {
        return Ok((pid, format!("Dashboard is already running (PID {pid}).")));
    }

    // Check npm is available
    let npm_check = Command::new("npm").arg("--version").output();
    if npm_check.is_err() || !npm_check.unwrap().status.success() {
        anyhow::bail!(
            "npm is not installed or not in PATH. \
             Install Node.js to use the dashboard dev server."
        );
    }

    // Resolve the project root
    let project_root = find_project_root().context(
        "Could not find the Mika project root (directory containing package.json with dev:dashboard script). \
         Run this command from within the Mika source tree."
    )?;

    // Read dashboard token from config (if available)
    let home = home::resolve_home_dir()?;
    mika_common::dotenv::load_dotenv(&home);
    let token = std::env::var("MIKA_DASHBOARD_TOKEN")
        .or_else(|_| std::env::var("MIKA_INTERNAL_TOKEN"))
        .ok();

    // Spawn the dev server as a background process
    let mut cmd = Command::new("npm");
    cmd.arg("run")
        .arg("dev:dashboard")
        .current_dir(&project_root);

    if let Some(t) = token {
        cmd.env("VITE_MIKA_DASHBOARD_TOKEN", t);
    }

    // Detach the process: redirect stdout/stderr to /dev/null
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null());

    let child = cmd
        .spawn()
        .context("Failed to start dashboard dev server")?;
    let pid = child.id();

    // Write PID file
    let pid_path = pid_file_path()?;
    fs::write(&pid_path, pid.to_string())
        .with_context(|| format!("Failed to write PID file: {}", pid_path.display()))?;

    Ok((
        pid,
        format!("Dashboard dev server started (PID {pid}).\n  URL: {DASHBOARD_URL}"),
    ))
}

/// Stop the dashboard process by PID.
///
/// Shared between CLI `stop` and TUI `/dashboard stop` handler.
pub fn stop_dashboard() -> String {
    let Some(pid) = read_pid() else {
        return "Dashboard is not running.".to_string();
    };

    // Send SIGTERM (signal 15)
    send_signal(pid, 15);

    // Wait briefly for the process to exit
    for _ in 0..50 {
        if !is_process_alive(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Clean up PID file
    if let Ok(path) = pid_file_path() {
        let _ = fs::remove_file(&path);
    }

    if is_process_alive(pid) {
        format!(
            "Dashboard process (PID {pid}) did not stop gracefully. You may need to kill it manually."
        )
    } else {
        "Dashboard stopped.".to_string()
    }
}

async fn start() -> Result<()> {
    let (_pid, message) = start_dashboard_process()?;
    println!("{message}");
    Ok(())
}

fn stop() -> Result<()> {
    let message = stop_dashboard();
    println!("{message}");
    Ok(())
}

fn status() -> Result<()> {
    if let Some(pid) = read_pid() {
        println!("Dashboard is running (PID {pid}).");
        println!("  URL: {DASHBOARD_URL}");
    } else {
        println!("Dashboard is not running.");
        println!("  Start with: mika dashboard start");
    }
    Ok(())
}

fn open() -> Result<()> {
    // Try to open in browser using platform-appropriate command
    let url = if read_pid().is_some() {
        DASHBOARD_URL
    } else {
        // If dev server is not running, try embedded dashboard
        "http://localhost:8080/dashboard/"
    };

    #[cfg(target_os = "linux")]
    let result = Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd").args(["/c", "start", url]).spawn();

    match result {
        Ok(_) => println!("Opening {url} in browser..."),
        Err(e) => println!("Could not open browser: {e}\n  URL: {url}"),
    }
    Ok(())
}

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
fn pid_file_path() -> Result<PathBuf> {
    let home = home::resolve_home_dir()?;
    Ok(home.join("dashboard.pid"))
}

/// Check if a process with the given PID is alive via /proc on Linux.
fn is_process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Send a signal to a process.
fn send_signal(pid: u32, signal: i32) {
    // Use std::process::Command to send signal via kill(1) utility
    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .output();
}

/// Read and validate the PID file. Returns `Some(pid)` if the process is alive.
fn read_pid() -> Option<u32> {
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

async fn start() -> Result<()> {
    // Check if already running
    if let Some(pid) = read_pid() {
        println!("Dashboard is already running (PID {pid}).");
        println!("  URL: {DASHBOARD_URL}");
        return Ok(());
    }

    // Check npm is available
    let npm_check = Command::new("npm").arg("--version").output();
    if npm_check.is_err() || !npm_check.unwrap().status.success() {
        anyhow::bail!(
            "npm is not installed or not in PATH. \
             Install Node.js to use the dashboard dev server."
        );
    }

    // Resolve the project root (where package.json lives).
    // Walk up from the current directory looking for package.json with "dev:dashboard" script.
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

    println!("Dashboard dev server started (PID {pid}).");
    println!("  URL: {DASHBOARD_URL}");
    Ok(())
}

fn stop() -> Result<()> {
    let pid_path = pid_file_path()?;

    let Some(pid) = read_pid() else {
        println!("Dashboard is not running.");
        return Ok(());
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
    let _ = fs::remove_file(&pid_path);

    if is_process_alive(pid) {
        println!(
            "Dashboard process (PID {pid}) did not stop gracefully. You may need to kill it manually."
        );
    } else {
        println!("Dashboard stopped.");
    }
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

/// Walk up from the current directory to find the Mika project root.
fn find_project_root() -> Option<PathBuf> {
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

/// Check if the dashboard dev server is running (used by TUI for status polling).
pub fn is_dashboard_running() -> bool {
    read_pid().is_some()
}

/// Public accessor for the PID file path (used by TUI handler).
pub fn pid_file_path_pub() -> Result<PathBuf> {
    pid_file_path()
}

/// Public accessor for reading the live PID (used by TUI handler).
pub fn read_pid_pub() -> Option<u32> {
    read_pid()
}

/// Public accessor for finding the project root (used by TUI handler).
pub fn find_project_root_pub() -> Option<PathBuf> {
    find_project_root()
}

/// Stop the dashboard process by PID (used by TUI handler).
pub fn stop_dashboard(pid: u32) {
    send_signal(pid, 15);
    // Wait briefly for process to exit
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
}

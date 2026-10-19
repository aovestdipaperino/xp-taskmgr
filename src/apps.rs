//! Running foreground applications, the macOS analogue of XP's task list.

use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    NSWorkspace,
};

/// A regular (Dock visible) application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppTask {
    pub pid: i32,
    pub name: String,
    pub responding: bool,
}

/// Lists regular applications, excluding this process.
pub fn running_apps() -> Vec<AppTask> {
    let own_pid = std::process::id() as i32;
    let apps = NSWorkspace::sharedWorkspace().runningApplications();
    let mut tasks: Vec<AppTask> = apps
        .iter()
        .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .filter(|app| app.processIdentifier() != own_pid)
        .map(|app| AppTask {
            pid: app.processIdentifier(),
            name: app
                .localizedName()
                .map_or_else(String::new, |n| n.to_string()),
            responding: !app.isTerminated(),
        })
        .collect();
    tasks.sort_by_key(|t| t.name.to_lowercase());
    tasks
}

fn find(pid: i32) -> Option<objc2::rc::Retained<NSRunningApplication>> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
}

/// Brings the application to the front.
pub fn switch_to(pid: i32) {
    if let Some(app) = find(pid) {
        app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    }
}

/// Asks the application to quit, as "End Task" did.
pub fn end_task(pid: i32) {
    if let Some(app) = find(pid) {
        app.terminate();
    }
}

/// Opens an application, document, folder or URL, like XP's Run dialog.
///
/// # Errors
///
/// Returns the `open` tool's error output when the target cannot be opened.
pub fn run(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Ok(());
    }
    let mut cmd = std::process::Command::new("/usr/bin/open");
    let looks_like_path = target.contains('/') || target.contains(':');
    if !looks_like_path {
        cmd.arg("-a");
    }
    let out = cmd.arg(target).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Sends a signal to a process; returns false when it was refused.
pub fn kill(pid: u32, force: bool) -> bool {
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: kill has no memory safety preconditions.
    unsafe { libc::kill(pid as libc::pid_t, sig) == 0 }
}

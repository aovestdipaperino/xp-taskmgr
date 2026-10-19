//! Running foreground applications, the macOS analogue of XP's task list.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSScreen,
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

/// Brings only the application's front window forward, unlike [`switch_to`].
pub fn bring_to_front(pid: i32) {
    if let Some(app) = find(pid) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}

/// Minimizes every window of the application.
///
/// Goes through System Events, so the first use asks for Accessibility access.
pub fn minimize(pid: i32) {
    run_system_events(format!(
        "tell (first process whose unix id is {pid}) to \
         set value of attribute \"AXMinimized\" of every window to true"
    ));
}

/// Restores the application's front window and fills the screen's visible
/// area with it, the closest macOS has to an XP maximized window.
pub fn maximize(pid: i32) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let screens = NSScreen::screens(mtm);
    let (Some(primary), Some(main)) = (screens.firstObject(), NSScreen::mainScreen(mtm)) else {
        return;
    };
    // AppKit frames grow upwards from the primary screen's bottom edge, while
    // Accessibility positions grow downwards from its top edge.
    let area = main.visibleFrame();
    let top = primary.frame().size.height - (area.origin.y + area.size.height);
    let (x, y, w, h) = (
        area.origin.x as i64,
        top as i64,
        area.size.width as i64,
        area.size.height as i64,
    );
    run_system_events(format!(
        "tell (first process whose unix id is {pid})
            set value of attribute \"AXMinimized\" of every window to false
            set position of window 1 to {{{x}, {y}}}
            set size of window 1 to {{{w}, {h}}}
        end tell"
    ));
}

/// Runs a script inside `tell application "System Events"` off the main
/// thread; failures (no window, Accessibility denied) are ignored.
fn run_system_events(script: String) {
    std::thread::spawn(move || {
        let script = format!("tell application \"System Events\"\n{script}\nend tell");
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .output();
    });
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

/// XP's priority classes, mapped onto Unix nice values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Realtime,
    High,
    AboveNormal,
    Normal,
    BelowNormal,
    Low,
}

impl Priority {
    /// In the order of XP's Set Priority submenu.
    pub const ALL: [Priority; 6] = [
        Priority::Realtime,
        Priority::High,
        Priority::AboveNormal,
        Priority::Normal,
        Priority::BelowNormal,
        Priority::Low,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Priority::Realtime => "Realtime",
            Priority::High => "High",
            Priority::AboveNormal => "AboveNormal",
            Priority::Normal => "Normal",
            Priority::BelowNormal => "BelowNormal",
            Priority::Low => "Low",
        }
    }

    fn nice(self) -> i32 {
        match self {
            Priority::Realtime => -20,
            Priority::High => -10,
            Priority::AboveNormal => -5,
            Priority::Normal => 0,
            Priority::BelowNormal => 5,
            Priority::Low => 20,
        }
    }
}

/// The priority class closest to the process's nice value.
pub fn priority(pid: u32) -> Option<Priority> {
    // SAFETY: getpriority has no memory safety preconditions; errno is
    // cleared first because -1 is also a valid nice value.
    let nice = unsafe {
        *libc::__error() = 0;
        let nice = libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t);
        if nice == -1 && *libc::__error() != 0 {
            return None;
        }
        nice
    };
    Priority::ALL
        .into_iter()
        .min_by_key(|p| (p.nice() - nice).abs())
}

/// Renices the process; raising priority needs root, as XP needed rights.
///
/// # Errors
///
/// Returns the OS error, typically permission denied.
pub fn set_priority(pid: u32, priority: Priority) -> std::io::Result<()> {
    // SAFETY: setpriority has no memory safety preconditions.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, priority.nice()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

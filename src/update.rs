//! Self-update from the latest GitHub release.
//!
//! Fetches with `reqwest` and shells out to `hdiutil` and `ditto` to mount and
//! copy the image. All functions block; call them from
//! the background executor.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

pub const REPO_URL: &str = "https://github.com/aovestdipaperino/xp-taskmgr";

/// The persisted option that arms the background update check.
pub const AUTO_UPDATE: &str = "AutoUpdate";

/// How often the background check looks for a release.
pub const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

const APP_NAME: &str = "Task Manager.app";

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{program} failed: {}", err.trim()))
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("xp-taskmgr/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

/// The newest release version, without the leading `v`.
///
/// `/releases/latest` redirects to `/releases/tag/<tag>`, so the final URL
/// carries the tag and no API call or JSON parsing is needed.
pub fn latest_version() -> Result<String, String> {
    let resp = client()?
        .get(format!("{REPO_URL}/releases/latest"))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    resp.url()
        .path()
        .rsplit_once("/tag/")
        .map(|(_, tag)| tag.trim_start_matches('v').to_owned())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "No release found".to_owned())
}

fn parts(version: &str) -> Vec<u64> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// True when `latest` is a higher version than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (mut a, mut b) = (parts(latest), parts(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// The `.app` bundle the running binary lives in, if it is bundled at all.
fn current_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| "Not running from an app bundle".to_owned())
}

/// Detaches the mounted image and removes the scratch directory on drop.
struct Scratch {
    dir: PathBuf,
    mount: PathBuf,
    mounted: bool,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.mounted {
            let _ = run(
                "/usr/bin/hdiutil",
                &["detach", "-force", &self.mount.to_string_lossy()],
            );
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// True while an update is downloading or swapping in, so the automatic check
/// and the About box button cannot both work on the same bundle.
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Held for as long as an update owns the bundle.
#[derive(Debug)]
struct Claim;

impl Drop for Claim {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::Relaxed);
    }
}

/// Claims the update path; `None` when an update is already running.
fn claim() -> Option<Claim> {
    // Not `then_some(Claim)`: the temporary would be built and dropped on the
    // branch where no claim was taken, freeing somebody else's.
    if INSTALLING.swap(true, Ordering::SeqCst) {
        None
    } else {
        Some(Claim)
    }
}

/// A release downloaded and staged next to the running bundle, waiting for the
/// user. Dropping it without applying discards the staged copy.
#[derive(Debug)]
pub struct Staged {
    staged: Option<PathBuf>,
    _claim: Claim,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(staged) = &self.staged {
            let _ = std::fs::remove_dir_all(staged);
        }
    }
}

/// Downloads `version`'s disk image and copies the app inside it next to the
/// running bundle. Blocks; call it from the background executor. Nothing is
/// touched until `apply`.
pub fn download(version: &str) -> Result<Staged, String> {
    let claim = claim().ok_or("An update is already in progress")?;
    let bundle = current_bundle()?;
    let dir = std::env::temp_dir().join(format!("xp-taskmgr-update-{}", std::process::id()));
    let mount = dir.join("mnt");
    std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
    let mut scratch = Scratch {
        dir: dir.clone(),
        mount: mount.clone(),
        mounted: false,
    };

    let dmg = dir.join("update.dmg");
    let url = format!("{REPO_URL}/releases/download/v{version}/XP-Task-Manager-{version}.dmg");
    let mut resp = client()?
        .get(&url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("Download failed: {e}"))?;
    let mut file = std::fs::File::create(&dmg).map_err(|e| e.to_string())?;
    resp.copy_to(&mut file)
        .map_err(|e| format!("Download failed: {e}"))?;
    drop(file);
    run(
        "/usr/bin/hdiutil",
        &[
            "attach",
            "-nobrowse",
            "-readonly",
            "-noverify",
            "-mountpoint",
            &mount.to_string_lossy(),
            &dmg.to_string_lossy(),
        ],
    )?;
    scratch.mounted = true;

    // Stage next to the live bundle so the final swap is a same-volume rename.
    let staged = bundle.with_extension("app.update");
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged)
        .map_err(|e| format!("Could not write next to the app (is it writable?): {e}"))?;
    run(
        "/usr/bin/ditto",
        &[
            &mount.join(APP_NAME).to_string_lossy(),
            &staged.to_string_lossy(),
        ],
    )
    .map_err(|e| format!("Could not write next to the app (is it writable?): {e}"))?;

    Ok(Staged {
        staged: Some(staged),
        _claim: claim,
    })
}

/// Swaps a staged update in over the running bundle and schedules the relaunch.
/// The caller should quit afterwards.
pub fn apply(staged: &mut Staged) -> Result<(), String> {
    let path = staged.staged.take().ok_or("Nothing staged")?;
    let bundle = current_bundle()?;
    let old = bundle.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&old);

    std::fs::rename(&bundle, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&path, &bundle) {
        let _ = std::fs::rename(&old, &bundle);
        // Put the copy back so a retry does not need a fresh download.
        if std::fs::create_dir_all(&path).is_ok() {
            let _ = run(
                "/usr/bin/ditto",
                &[&bundle.to_string_lossy(), &path.to_string_lossy()],
            );
        }
        staged.staged = Some(path);
        return Err(e.to_string());
    }
    let _ = std::fs::remove_dir_all(&old);

    // Relaunch once this process has had a moment to exit.
    Command::new("/bin/sh")
        .args(["-c", "sleep 1; /usr/bin/open -n \"$0\""])
        .arg(&bundle)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Staged, claim, is_newer};

    #[test]
    fn discarding_a_staged_update_frees_the_bundle() {
        // Cancelling the offer must remove the staged copy and let a later
        // check download again.
        let dir = std::env::temp_dir().join(format!("xp-taskmgr-staged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let taken = claim().expect("the update path starts free");
        drop(Staged {
            staged: Some(dir.clone()),
            _claim: taken,
        });
        assert!(!dir.exists(), "the staged copy should be discarded");
        assert!(claim().is_some(), "the update path should be free again");
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
    }
}

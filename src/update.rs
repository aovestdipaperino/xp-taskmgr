//! Self-update from the latest GitHub release.
//!
//! Fetches with `reqwest` and shells out to `hdiutil` and `ditto` to mount and
//! copy the image. All functions block; call them from
//! the background executor.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO_URL: &str = "https://github.com/aovestdipaperino/xp-taskmgr";

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

/// Downloads `version`'s disk image and replaces the running bundle with the
/// app inside it, then schedules a relaunch. The caller should quit afterwards.
pub fn install(version: &str) -> Result<(), String> {
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
    let old = bundle.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&staged);
    let _ = std::fs::remove_dir_all(&old);
    run(
        "/usr/bin/ditto",
        &[
            &mount.join(APP_NAME).to_string_lossy(),
            &staged.to_string_lossy(),
        ],
    )
    .map_err(|e| format!("Could not write next to the app (is it writable?): {e}"))?;

    std::fs::rename(&bundle, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&staged, &bundle) {
        let _ = std::fs::rename(&old, &bundle);
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
    use super::is_newer;

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

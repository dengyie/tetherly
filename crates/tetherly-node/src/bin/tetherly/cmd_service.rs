// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `tetherly service`: install/uninstall/report the OS-level autostart entry
//! for the daemon (production 2026-11-25: a Mac reboot left only the shell
//! autostarted — the daemon had no registration path at all, so every paired
//! peer went offline until someone ran `tetherly run` by hand).
//!
//! Platform targets:
//! - **macOS**: a LaunchAgent plist under `~/Library/LaunchAgents` with
//!   `RunAtLoad` + `KeepAlive`, loaded via `launchctl bootstrap gui/<uid>`.
//!   `KeepAlive` is what makes crash recovery work; `RunAtLoad` covers login.
//! - **Windows**: a per-user scheduled task (`Register-ScheduledTask`,
//!   logon trigger + restart-on-failure) — the same shape as the manual
//!   `TetherlySystemLauncher` task the deploy chain assumed, but registered
//!   by the software instead of by hand (the manual task does not survive
//!   reinstalls or a new machine).

use super::*;
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Debug, Clone, Copy)]
pub(crate) enum ServiceAction {
    /// Install the daemon autostart entry (idempotent; migrates legacy manual entries).
    Install,
    /// Remove the daemon autostart entry.
    Uninstall,
    /// Print whether the autostart entry is installed and what it runs.
    Status,
}

/// `tetherly service` -- Service arm of the top-level dispatch.
pub(crate) async fn service(action: ServiceAction, json: bool) -> Result<()> {
    match action {
        ServiceAction::Install => install(json)?,
        ServiceAction::Uninstall => uninstall(json)?,
        ServiceAction::Status => status(json)?,
    }
    Ok(())
}

// ---- shared helpers ----

/// The label/file name of the managed autostart entry. One name on both
/// platforms keeps `status` trivial.
#[cfg(target_os = "macos")]
pub(crate) const SERVICE_LABEL: &str = "dev.tetherly.tetherly";
#[cfg(windows)]
pub(crate) const SERVICE_TASK_NAME: &str = "TetherlyDaemon";

/// The daemon executable this process is running from. The LaunchAgent /
/// scheduled task points HERE, so a dev build registers its own debug
/// binary and a packaged build registers the .app binary — no path
/// guessing (`lifecycle.rs` uses the same trick for self-restart).
fn daemon_exe() -> Result<PathBuf> {
    std::env::current_exe().map_err(|e| anyhow::anyhow!("current_exe: {e}"))
}

// ---- macOS ----

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    const LAUNCH_AGENTS: &str = "Library/LaunchAgents";

    fn plist_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        Ok(PathBuf::from(home).join(LAUNCH_AGENTS).join(format!("{SERVICE_LABEL}.plist")))
    }

    /// Render the managed plist body. Pure function so tests can pin the
    /// exact keys without touching the filesystem.
    fn render_plist(exe: &Path, log_path: &Path) -> String {
        let exe = exe.display();
        let log_path = log_path.display();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<!-- Installed by `tetherly service install`. Remove with `tetherly service uninstall`. -->
<dict>
    <key>Label</key>
    <string>{SERVICE_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ProcessType</key>
    <string>Background</string>
    <key>StandardOutPath</key>
    <string>{log_path}</string>
    <key>StandardErrorPath</key>
    <string>{log_path}</string>
</dict>
</plist>
"#
        )
    }

    /// True if `path` is a hand-made plist that runs a Tetherly daemon
    /// (`.../Tetherly[ ...] run`), i.e. a legacy entry to migrate. Managed
    /// entries (`dev.tetherly.tetherly.plist`) are never legacy.
    fn is_legacy_tetherly_plist(body: &str) -> bool {
        let managed = format!("<string>{SERVICE_LABEL}</string>");
        !body.contains(&managed) && body.contains("Tetherly") && body.contains(">run<")
    }

    fn launchctl(args: &[&str]) -> Result<String> {
        let out = std::process::Command::new("launchctl")
            .args(args)
            .output()
            .map_err(|e| anyhow::anyhow!("launchctl spawn failed: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            anyhow::bail!("launchctl {:?} failed: {}", args, text.trim());
        }
        Ok(text)
    }

    fn gui_domain() -> Result<String> {
        let uid = unsafe { libc::getuid() };
        Ok(format!("gui/{uid}"))
    }

    pub(crate) fn install(json: bool) -> Result<()> {
        let exe = daemon_exe()?;
        let data_dir = tetherly_node::store::default_data_dir();
        std::fs::create_dir_all(&data_dir)?;
        let log_path = data_dir.join("launchd.log");
        let body = render_plist(&exe, &log_path);

        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Idempotent: identical content → only make sure the job is loaded.
        if path.is_file() {
            let existing = std::fs::read_to_string(&path)?;
            if existing == body {
                // Loaded-state repair: a plist that exists but is not
                // bootstrapped would never fire; bootstrap reports an error
                // when already loaded, which we treat as success.
                match launchctl(&["bootstrap", &gui_domain()?, &path.to_string_lossy()]) {
                    Ok(_) => {}
                    Err(e) if e.to_string().contains("already bootstrapped") => {}
                    // launchctl's "already loaded" shape (observed 2026-11-25:
                    // `Bootstrap failed: 5: Input/output error` on a live job).
                    Err(e) if e.to_string().contains("Input/output error") => {}
                    Err(e) => return Err(e),
                }
                report_status_line(json, "already-installed", &exe)?;
                return Ok(());
            }
        }

        // Migrate legacy hand-made daemon plists (e.g. `cn.mango.tetherly`
        // from the 2026-11-25 manual fix) so two KeepAlive agents do not
        // fight over the same `run` binary after the next login.
        let agents_dir = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(LAUNCH_AGENTS);
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p == path || !p.extension().is_some_and(|e| e == "plist") {
                    continue;
                }
                if let Ok(other) = std::fs::read_to_string(&p) {
                    if is_legacy_tetherly_plist(&other) {
                        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        // Bootout by label when the file's Label matches its name.
                        let legacy_label = name.trim_end_matches(".plist");
                        let _ = launchctl(&["bootout", &format!("{}/{}", gui_domain()?, legacy_label)]);
                        std::fs::remove_file(&p).map_err(|e| {
                            anyhow::anyhow!("cannot remove legacy plist {}: {e}", p.display())
                        })?;
                        if !json {
                            println!("migrated legacy autostart entry: {name}");
                        }
                    }
                }
            }
        }

        // Write + bootstrap. The tmp+rename keeps a crash mid-write from
        // leaving a truncated plist (the store.rs convention).
        let tmp = path.with_extension("plist.tmp");
        std::fs::write(&tmp, &body)?;
        std::fs::rename(&tmp, &path)?;
        match launchctl(&["bootstrap", &gui_domain()?, &path.to_string_lossy()]) {
            Ok(_) => {}
            // Already loaded from a previous incarnation of this plist.
            Err(e) if e.to_string().contains("already bootstrapped") => {}
            Err(e) if e.to_string().contains("Input/output error") => {}
            Err(e) => return Err(e),
        }
        report_status_line(json, "installed", &exe)
    }

    pub(crate) fn uninstall(json: bool) -> Result<()> {
        let path = plist_path()?;
        if !path.is_file() {
            if !json {
                println!("not installed ({})", path.display());
            }
            return Ok(());
        }
        match launchctl(&["bootout", &format!("{}/{}", gui_domain()?, SERVICE_LABEL)]) {
            Ok(_) => {}
            // Not currently loaded is fine — the goal is that it stops firing.
            Err(_) => {}
        }
        std::fs::remove_file(&path)?;
        report_status_line(json, "uninstalled", &daemon_exe()?)
    }

    pub(crate) fn status(json: bool) -> Result<()> {
        let path = plist_path()?;
        let installed = path.is_file();
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "platform": "macos",
                    "label": SERVICE_LABEL,
                    "installed": installed,
                    "plist": path.display().to_string(),
                    "exe": daemon_exe()?.display().to_string(),
                })
            );
            return Ok(());
        }
        if installed {
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            let exe = body
                .lines()
                .skip_while(|l| !l.contains("ProgramArguments"))
                .skip(2)
                .find_map(|l| l.trim().strip_prefix("<string>"))
                .map(|s| s.trim_end_matches("</string>").to_string())
                .unwrap_or_default();
            println!("installed: {}", path.display());
            println!("runs: {exe} run");
        } else {
            println!("not installed ({})", path.display());
        }
        Ok(())
    }

    fn report_status_line(json: bool, what: &str, exe: &Path) -> Result<()> {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "platform": "macos",
                    "result": what,
                    "plist": plist_path()?.display().to_string(),
                    "exe": exe.display().to_string(),
                })
            );
        } else {
            let path = plist_path()?;
            println!("{what}: {}", path.display());
            println!("runs: {} run (RunAtLoad + KeepAlive)", exe.display());
        }
        Ok(())
    }

    // ---- tests ----
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn plist_has_keepalive_and_run_args() {
            let body = render_plist(Path::new("/Applications/Tetherly.app/Contents/MacOS/Tetherly"),
                Path::new("/tmp/launchd.log"));
            assert!(body.contains("<string>/Applications/Tetherly.app/Contents/MacOS/Tetherly</string>"));
            assert!(body.contains("<string>run</string>"));
            assert!(body.contains("<key>KeepAlive</key>"));
            assert!(body.contains("<key>RunAtLoad</key>"));
        }

        #[test]
        fn legacy_detection_matches_manual_entry() {
            // The 2026-11-25 hand-made entry shape.
            let manual = r#"<key>Label</key><string>cn.mango.tetherly</string>
<key>ProgramArguments</key><array><string>/Applications/Tetherly.app/Contents/MacOS/Tetherly</string><string>run</string></array>"#;
            assert!(is_legacy_tetherly_plist(manual));
            // Managed entries are never legacy.
            assert!(!is_legacy_tetherly_plist(&format!(
                "<string>{SERVICE_LABEL}</string><string>run</string>"
            )));
            // The shell's autostart plist runs no bare `run` argument.
            let shell = r#"<key>ProgramArguments</key><array><string>/Applications/Tetherly Client.app/Contents/MacOS/tetherly-shell</string></array>"#;
            assert!(!is_legacy_tetherly_plist(shell));
        }
    }
}

#[cfg(target_os = "macos")]
use macos::{install, status, uninstall};

// ---- Windows ----

#[cfg(windows)]
mod windows_impl {
    use super::*;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn powershell(script: &str) -> Result<String> {
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| anyhow::anyhow!("powershell spawn failed: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            anyhow::bail!("powershell failed: {}", text.trim());
        }
        Ok(text)
    }

    /// Register the per-user logon task. `RunLevel Highest` mirrors the
    /// manual `TetherlySystemLauncher` chain's assumption; the restart
    /// interval keeps a crashed daemon coming back within 5 minutes.
    pub(crate) fn install(json: bool) -> Result<()> {
        let exe = daemon_exe()?;
        let action = format!("\"{}\" run", exe.display());
        // Two-step build via New-ScheduledTaskAction keeps quoting sane.
        let script = format!(
            r#"$a = New-ScheduledTaskAction -Execute '{exe}' -Argument 'run'
$t = New-ScheduledTaskTrigger -AtLogOn
$s = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 5) -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable
Register-ScheduledTask -TaskName '{name}' -Action $a -Trigger $t -Settings $s -RunLevel Highest -Force | Out-Null"#,
            exe = exe.display(),
            name = SERVICE_TASK_NAME,
        );
        powershell(&script)?;
        if !json {
            println!("installed: scheduled task '{SERVICE_TASK_NAME}' (logon trigger, restart on failure)");
            println!("runs: {} run", exe.display());
        } else {
            println!(
                "{}",
                serde_json::json!({"platform": "windows", "result": "installed", "task": SERVICE_TASK_NAME})
            );
        }
        Ok(())
    }

    pub(crate) fn uninstall(json: bool) -> Result<()> {
        powershell(&format!(
            "Unregister-ScheduledTask -TaskName '{SERVICE_TASK_NAME}' -Confirm:$false"
        ))?;
        if !json {
            println!("uninstalled: {SERVICE_TASK_NAME}");
        }
        Ok(())
    }

    pub(crate) fn status(json: bool) -> Result<()> {
        let info = powershell(&format!(
            "$t = Get-ScheduledTask -TaskName '{SERVICE_TASK_NAME}' -ErrorAction SilentlyContinue; \
             if ($t) {{ $t.State }} else {{ 'NotFound' }}"
        ))?;
        let state = info.trim().to_string();
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "platform": "windows",
                    "task": SERVICE_TASK_NAME,
                    "state": state,
                })
            );
        } else if state == "NotFound" {
            println!("not installed ({SERVICE_TASK_NAME})");
        } else {
            println!("installed: {SERVICE_TASK_NAME} (state: {state})");
        }
        Ok(())
    }
}

#[cfg(windows)]
use windows_impl::{install, status, uninstall};

// ---- other platforms: honest refusal, no silent fake ----

#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported {
    use super::*;
    fn install(_json: bool) -> Result<()> {
        anyhow::bail!("`tetherly service install` is not implemented for this platform; use a user session manager (e.g. systemd user units)")
    }
    fn uninstall(_json: bool) -> Result<()> {
        anyhow::bail!("`tetherly service uninstall` is not implemented for this platform")
    }
    fn status(_json: bool) -> Result<()> {
        anyhow::bail!("`tetherly service status` is not implemented for this platform")
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
use unsupported::{install, status, uninstall};

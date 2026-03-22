use crate::types::{CheckResult, SystemContext};
use std::fs;
use std::process::Command;

pub fn check(ctx: &SystemContext) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // Read environment of the FrankenPHP process (find its PID first)
    let frankenphp_env = find_frankenphp_env();

    // Try to find the systemd service override path for auto-fixing
    let service_override = find_frankenphp_service_override();

    // GODEBUG=cgocheck=0
    match &frankenphp_env {
        Some(env) if env.contains("GODEBUG=cgocheck=0") => {
            results.push(CheckResult::ok("GODEBUG", "cgocheck=0 set"));
        }
        Some(_) | None => {
            let detail = if frankenphp_env.is_some() {
                "cgocheck=0 not found in FrankenPHP process environment"
            } else {
                "Could not read FrankenPHP process environment — ensure GODEBUG=cgocheck=0 is set"
            };

            let mut result = CheckResult::warn("GODEBUG", detail);

            if let Some(ref override_path) = service_override {
                result = result.with_fix(
                    "Add GODEBUG=cgocheck=0 to FrankenPHP service",
                    override_path,
                    "Environment=GODEBUG=cgocheck=0",
                );
            } else {
                result = result.with_fix(
                    "Add GODEBUG=cgocheck=0 to Forge site environment",
                    "Forge Site → Environment",
                    "GODEBUG=cgocheck=0",
                );
            }

            results.push(result);
        }
    }

    // GOMEMLIMIT
    let recommended_gb = (ctx.php_ram_budget_mb as f64 / 1024.0).round().max(1.0) as u64;
    match &frankenphp_env {
        Some(env) => {
            if let Some(val) = env
                .lines()
                .find(|l| l.starts_with("GOMEMLIMIT="))
                .map(|l| l.trim_start_matches("GOMEMLIMIT="))
            {
                results.push(CheckResult::ok("GOMEMLIMIT", format!("{val}")));
            } else {
                let mut result = CheckResult::warn(
                    "GOMEMLIMIT",
                    format!("Not set — recommend {recommended_gb}GiB based on PHP RAM budget"),
                );

                if let Some(ref override_path) = service_override {
                    result = result.with_fix(
                        format!("Set GOMEMLIMIT={recommended_gb}GiB"),
                        override_path,
                        format!("Environment=GOMEMLIMIT={recommended_gb}GiB"),
                    );
                } else {
                    result = result.with_fix(
                        format!("Set GOMEMLIMIT={recommended_gb}GiB"),
                        "Forge Site → Environment",
                        format!("GOMEMLIMIT={recommended_gb}GiB"),
                    );
                }

                results.push(result);
            }
        }
        None => {
            results.push(CheckResult::warn(
                "GOMEMLIMIT",
                format!("Could not check — recommend {recommended_gb}GiB"),
            ));
        }
    }

    results
}

/// Find the systemd service override path for FrankenPHP.
/// Returns a path like `/etc/systemd/system/frankenphp.service.d/override.conf`
/// if we can identify the service unit.
fn find_frankenphp_service_override() -> Option<String> {
    let pid = find_frankenphp_pid()?;

    // Use systemctl to find the unit name from the PID
    let output = Command::new("systemctl")
        .args(["status", &pid.to_string()])
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    // First line: ● unit-name.service - Description
    let first_line = text.lines().next()?;
    let unit = first_line
        .split_whitespace()
        .find(|w| w.ends_with(".service"))?;

    Some(format!("/etc/systemd/system/{unit}.d/override.conf"))
}

fn find_frankenphp_pid() -> Option<u32> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let pid_str = entry.file_name();
        let pid_str = pid_str.to_string_lossy().to_string();
        if let Ok(pid) = pid_str.parse::<u32>() {
            if let Ok(comm) = fs::read_to_string(format!("/proc/{pid}/comm")) {
                if comm.trim() == "frankenphp" {
                    return Some(pid);
                }
            }
        }
    }
    None
}

fn find_frankenphp_env() -> Option<String> {
    let pid = find_frankenphp_pid()?;
    let env = fs::read_to_string(format!("/proc/{pid}/environ")).ok()?;
    // /proc/pid/environ uses null bytes as separators
    Some(env.replace('\0', "\n"))
}

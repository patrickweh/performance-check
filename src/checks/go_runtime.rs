use crate::types::{CheckResult, SystemContext};
use std::fs;

pub fn check(ctx: &SystemContext) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // Read environment of the FrankenPHP process (find its PID first)
    let frankenphp_env = find_frankenphp_env();

    // GODEBUG=cgocheck=0
    match &frankenphp_env {
        Some(env) if env.contains("GODEBUG=cgocheck=0") => {
            results.push(CheckResult::ok("GODEBUG", "cgocheck=0 set"));
        }
        Some(_) => {
            results.push(
                CheckResult::warn(
                    "GODEBUG",
                    "cgocheck=0 not found in FrankenPHP process environment — set GODEBUG=cgocheck=0 in Forge site environment",
                )
                .with_fix(
                    "Add GODEBUG=cgocheck=0 to Forge site environment",
                    "Forge Site → Environment",
                    "GODEBUG=cgocheck=0",
                ),
            );
        }
        None => {
            results.push(CheckResult::warn(
                "GODEBUG",
                "Could not read FrankenPHP process environment — ensure GODEBUG=cgocheck=0 is set",
            ));
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
                results.push(
                    CheckResult::warn(
                        "GOMEMLIMIT",
                        format!("Not set — recommend {recommended_gb}GiB based on PHP RAM budget"),
                    )
                    .with_fix(
                        format!("Set GOMEMLIMIT={recommended_gb}GiB"),
                        "Forge Site → Environment",
                        format!("GOMEMLIMIT={recommended_gb}GiB"),
                    ),
                );
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

fn find_frankenphp_env() -> Option<String> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let pid_str = entry.file_name();
        let pid_str = pid_str.to_string_lossy().to_string();
        if pid_str.parse::<u32>().is_err() {
            continue;
        }
        let comm = fs::read_to_string(format!("/proc/{pid_str}/comm")).ok()?;
        if comm.trim() == "frankenphp" {
            let env = fs::read_to_string(format!("/proc/{pid_str}/environ")).ok()?;
            // /proc/pid/environ uses null bytes as separators
            return Some(env.replace('\0', "\n"));
        }
    }
    None
}

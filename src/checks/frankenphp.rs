use crate::types::CheckResult;
use std::path::Path;
use std::process::Command;

pub fn check(frankenphp_bin: &str) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // Binary exists and is executable
    if !Path::new(frankenphp_bin).exists() {
        results.push(CheckResult::fail(
            "FrankenPHP Binary",
            format!("{frankenphp_bin} not found"),
        ));
        return results;
    }

    results.push(CheckResult::ok(
        "FrankenPHP Binary",
        format!("{frankenphp_bin} found"),
    ));

    // Version
    match Command::new(frankenphp_bin).arg("version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if version.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                results.push(CheckResult::info("FrankenPHP Version", stderr));
            } else {
                results.push(CheckResult::info("FrankenPHP Version", version));
            }
        }
        Err(e) => {
            results.push(CheckResult::warn(
                "FrankenPHP Version",
                format!("Could not determine: {e}"),
            ));
        }
    }

    results
}

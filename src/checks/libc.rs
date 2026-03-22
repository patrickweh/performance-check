use crate::types::CheckResult;
use std::process::Command;

pub fn check() -> Vec<CheckResult> {
    let mut results = Vec::new();

    // Check if we're on musl or glibc via ldd --version
    let output = Command::new("ldd").arg("--version").output();

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_string()
                + &String::from_utf8_lossy(&out.stderr);
            let text_lower = text.to_lowercase();

            if text_lower.contains("musl") {
                results.push(CheckResult::fail(
                    "libc",
                    "musl detected — significantly slower for PHP-ZTS. Use a glibc-based distro (Debian, Ubuntu)",
                ));
            } else if text_lower.contains("glibc") || text_lower.contains("gnu") {
                results.push(CheckResult::ok("libc", "glibc"));
            } else {
                results.push(CheckResult::info(
                    "libc",
                    format!("Unknown: {}", text.lines().next().unwrap_or("?")),
                ));
            }
        }
        Err(_) => {
            // Fallback: check if /lib/ld-musl-* exists
            let musl_glob = std::fs::read_dir("/lib")
                .map(|entries| {
                    entries
                        .flatten()
                        .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl"))
                })
                .unwrap_or(false);

            if musl_glob {
                results.push(CheckResult::fail(
                    "libc",
                    "musl detected — significantly slower for PHP-ZTS",
                ));
            } else {
                results.push(CheckResult::ok("libc", "glibc (assumed)"));
            }
        }
    }

    // File descriptor limits
    let ulimit = Command::new("sh")
        .args(["-c", "ulimit -n"])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u64>()
                .ok()
        });

    match ulimit {
        Some(n) if n >= 65536 => {
            results.push(CheckResult::ok(
                "File Descriptors (ulimit -n)",
                format!("{n}"),
            ));
        }
        Some(n) => {
            results.push(CheckResult::warn(
                "File Descriptors (ulimit -n)",
                format!("{n} — recommend ≥65536 for high-traffic servers"),
            ));
        }
        None => {
            results.push(CheckResult::info(
                "File Descriptors",
                "Could not read ulimit",
            ));
        }
    }

    results
}

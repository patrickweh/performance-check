use crate::types::{CheckResult, SystemContext};
use std::path::Path;
use std::process::Command;

const REQUIRED_EXTENSIONS: &[&str] = &[
    "bcmath", "pdo", "pdo_mysql", "redis", "gd", "intl", "zip", "opcache",
];

pub fn check(frankenphp_bin: &str, php_ini: &str, ctx: &SystemContext) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // php.ini exists
    if Path::new(php_ini).exists() {
        results.push(CheckResult::ok("PHP-ZTS php.ini", format!("{php_ini} found")));
    } else {
        results.push(CheckResult::fail(
            "PHP-ZTS php.ini",
            format!("{php_ini} not found"),
        ));
    }

    // Extensions
    for ext in REQUIRED_EXTENSIONS {
        let loaded = php_eval(frankenphp_bin, &format!("echo extension_loaded('{ext}') ? '1' : '0';"));
        match loaded.as_deref() {
            Some("1") => {
                results.push(CheckResult::ok(
                    format!("PHP ext: {ext}"),
                    "loaded",
                ));
            }
            _ => {
                results.push(CheckResult::fail(
                    format!("PHP ext: {ext}"),
                    format!("not loaded — install and enable in {php_ini}"),
                ));
            }
        }
    }

    // OPcache settings
    check_ini_bool(frankenphp_bin, php_ini, "opcache.enable", true, &mut results);
    check_ini_bool(frankenphp_bin, php_ini, "opcache.validate_timestamps", false, &mut results);
    check_ini_min(frankenphp_bin, php_ini, "opcache.memory_consumption", 256, &mut results);
    check_ini_min(frankenphp_bin, php_ini, "opcache.max_accelerated_files", 20000, &mut results);
    check_ini_min(frankenphp_bin, php_ini, "opcache.interned_strings_buffer", 32, &mut results);

    // opcache.jit_buffer_size
    let jit = get_ini(frankenphp_bin, "opcache.jit_buffer_size");
    match jit.as_deref() {
        Some(v) if !v.is_empty() && v != "0" => {
            results.push(CheckResult::ok(
                "opcache.jit_buffer_size",
                format!("{v}"),
            ));
        }
        _ => {
            results.push(
                CheckResult::warn(
                    "opcache.jit_buffer_size",
                    "Not set — recommend 128M for Octane workers",
                )
                .with_fix(
                    "Set opcache.jit_buffer_size=128M",
                    php_ini,
                    "opcache.jit_buffer_size=128M",
                ),
            );
        }
    }

    // opcache.preload — should NOT be set with Octane
    let preload = get_ini(frankenphp_bin, "opcache.preload");
    match preload.as_deref() {
        Some(v) if !v.is_empty() => {
            results.push(CheckResult::warn(
                "opcache.preload",
                format!("Set to '{v}' — should NOT be set with Octane worker mode"),
            ));
        }
        _ => {
            results.push(CheckResult::ok("opcache.preload", "Not set (correct for Octane)"));
        }
    }

    // Realpath cache
    let rp_size = get_ini(frankenphp_bin, "realpath_cache_size");
    match rp_size.as_deref() {
        Some(v) if !v.is_empty() => {
            let numeric = parse_php_size(v);
            if numeric < 4096 * 1024 {
                results.push(
                    CheckResult::warn(
                        "realpath_cache_size",
                        format!("{v} — recommend ≥4096K for large apps"),
                    )
                    .with_fix(
                        "Set realpath_cache_size=4096K",
                        php_ini,
                        "realpath_cache_size=4096K",
                    ),
                );
            } else {
                results.push(CheckResult::ok("realpath_cache_size", v.to_string()));
            }
        }
        _ => {
            results.push(
                CheckResult::warn("realpath_cache_size", "Not set — recommend 4096K")
                    .with_fix("Set realpath_cache_size=4096K", php_ini, "realpath_cache_size=4096K"),
            );
        }
    }

    let rp_ttl = get_ini_int(frankenphp_bin, "realpath_cache_ttl");
    if rp_ttl < 300 {
        results.push(
            CheckResult::warn(
                "realpath_cache_ttl",
                format!("{rp_ttl} — recommend ≥600"),
            )
            .with_fix(
                "Set realpath_cache_ttl=600",
                php_ini,
                "realpath_cache_ttl=600",
            ),
        );
    } else {
        results.push(CheckResult::ok(
            "realpath_cache_ttl",
            format!("{rp_ttl}"),
        ));
    }

    // memory_limit
    let mem_limit = get_ini(frankenphp_bin, "memory_limit");
    match mem_limit.as_deref() {
        Some(v) if !v.is_empty() => {
            let bytes = parse_php_size(v);
            let mb = bytes / (1024 * 1024);
            if mb < 128 {
                results.push(CheckResult::warn(
                    "memory_limit",
                    format!("{v} — may be too low for Octane workers"),
                ));
            } else {
                results.push(CheckResult::ok("memory_limit", v.to_string()));
            }

            // Check if total worker memory could exceed budget
            let worker_total = mb as u64 * ctx.cpu_cores as u64;
            if worker_total > ctx.php_ram_budget_mb {
                results.push(CheckResult::warn(
                    "Worker Memory Risk",
                    format!(
                        "{} workers × {mb}MB = {worker_total}MB > {budget}MB PHP RAM budget",
                        ctx.cpu_cores,
                        budget = ctx.php_ram_budget_mb
                    ),
                ));
            }
        }
        _ => {
            results.push(CheckResult::warn("memory_limit", "Could not read"));
        }
    }

    results
}

fn php_eval(frankenphp_bin: &str, code: &str) -> Option<String> {
    Command::new(frankenphp_bin)
        .args(["php-cli", "-r", code])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn get_ini(frankenphp_bin: &str, key: &str) -> Option<String> {
    php_eval(frankenphp_bin, &format!("echo ini_get('{key}');"))
}

fn get_ini_int(frankenphp_bin: &str, key: &str) -> i64 {
    get_ini(frankenphp_bin, key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn check_ini_bool(
    frankenphp_bin: &str,
    php_ini: &str,
    key: &str,
    expected: bool,
    results: &mut Vec<CheckResult>,
) {
    let val = get_ini(frankenphp_bin, key);
    let is_on = matches!(val.as_deref(), Some("1") | Some("On") | Some("on"));
    if is_on == expected {
        results.push(CheckResult::ok(key, format!("{}", if is_on { "On" } else { "Off" })));
    } else {
        let expected_val = if expected { "1" } else { "0" };
        results.push(
            CheckResult::fail(
                key,
                format!(
                    "Expected {} — set {key}={expected_val} in {php_ini}",
                    if expected { "On" } else { "Off" }
                ),
            )
            .with_fix(
                format!("Set {key}={expected_val}"),
                php_ini,
                format!("{key}={expected_val}"),
            ),
        );
    }
}

fn check_ini_min(
    frankenphp_bin: &str,
    php_ini: &str,
    key: &str,
    min: i64,
    results: &mut Vec<CheckResult>,
) {
    let val = get_ini_int(frankenphp_bin, key);
    if val >= min {
        results.push(CheckResult::ok(key, format!("{val}")));
    } else {
        results.push(
            CheckResult::fail(
                key,
                format!("{val} — should be ≥{min}"),
            )
            .with_fix(
                format!("Set {key}={min}"),
                php_ini,
                format!("{key}={min}"),
            ),
        );
    }
}

fn parse_php_size(val: &str) -> u64 {
    let val = val.trim();
    if val.ends_with('G') || val.ends_with('g') {
        val[..val.len() - 1].parse::<u64>().unwrap_or(0) * 1024 * 1024 * 1024
    } else if val.ends_with('M') || val.ends_with('m') {
        val[..val.len() - 1].parse::<u64>().unwrap_or(0) * 1024 * 1024
    } else if val.ends_with('K') || val.ends_with('k') {
        val[..val.len() - 1].parse::<u64>().unwrap_or(0) * 1024
    } else {
        val.parse::<u64>().unwrap_or(0)
    }
}

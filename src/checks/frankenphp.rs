use crate::types::{CheckResult, SystemContext};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Configuration detected from the Caddy admin API.
#[derive(Debug, Default)]
struct AdminApiConfig {
    has_worker: bool,
    worker_count: Option<u32>,
    num_threads: Option<u32>,
    reachable: bool,
}

pub fn check(frankenphp_bin: &str, app_path: &str, ctx: &SystemContext) -> Vec<CheckResult> {
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

    // Parse Caddyfile for per-site configuration
    let caddyfile = find_caddyfile(app_path);
    let site_config = caddyfile
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|content| parse_site_config(&content, app_path));

    // Query the Caddy admin API for runtime configuration (most reliable for Octane)
    let admin_config = query_admin_api();

    // Worker Mode check
    check_worker_mode(&site_config, &admin_config, &mut results);

    // num_threads tuning check
    check_num_threads(&site_config, &admin_config, ctx, &mut results);

    // Log Level check
    if let Some(ref caddyfile_content) = caddyfile
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
    {
        check_log_level(caddyfile_content, &mut results);
    }

    // Symlink Resolution check
    check_symlink_root(app_path, &mut results);

    results
}

/// Parsed configuration for a specific site from the Caddyfile.
#[derive(Debug, Default)]
struct SiteConfig {
    has_worker: bool,
    worker_num: Option<u32>,
    num_threads: Option<u32>,
    has_php_server: bool,
}

/// Find the active Caddyfile.
///
/// Search order:
/// 1. --caddyfile flag from the running FrankenPHP process
/// 2. Well-known system paths
/// 3. Project root Caddyfile
fn find_caddyfile(app_path: &str) -> Option<String> {
    // Check running FrankenPHP process for --caddyfile or --config flag
    if let Some(path) = find_caddyfile_from_process() {
        return Some(path);
    }

    // Well-known paths + project root
    let project_caddyfile = format!("{app_path}/Caddyfile");
    let paths = [
        "/etc/caddy/Caddyfile",
        "/etc/frankenphp/Caddyfile",
        &project_caddyfile,
    ];
    for path in &paths {
        if Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    None
}

/// Extract --caddyfile or --config path from the running FrankenPHP/Caddy process.
fn find_caddyfile_from_process() -> Option<String> {
    let output = Command::new("ps")
        .args(["--no-headers", "-eo", "args"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if !line.contains("frankenphp") && !line.contains("caddy") {
            continue;
        }
        // Look for --caddyfile /path or --config /path
        let parts: Vec<&str> = line.split_whitespace().collect();
        for (i, part) in parts.iter().enumerate() {
            if (*part == "--caddyfile" || *part == "--config" || *part == "-c")
                && i + 1 < parts.len()
            {
                let path = parts[i + 1];
                if Path::new(path).exists() {
                    return Some(path.to_string());
                }
            }
        }
    }
    None
}

/// Query the Caddy admin API to detect running configuration.
///
/// This is the most reliable detection method for Octane/Forge setups
/// where the Caddyfile is generated dynamically and not on disk.
fn query_admin_api() -> AdminApiConfig {
    let mut config = AdminApiConfig::default();

    // Try common admin ports (2019 is Caddy default, Octane uses configurable admin port)
    let ports = [2019, 2020];

    for port in ports {
        let output = Command::new("curl")
            .args([
                "-s",
                "--connect-timeout",
                "1",
                "--max-time",
                "2",
                &format!("http://localhost:{port}/config/"),
            ])
            .output();

        let output = match output {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };

        let body = String::from_utf8_lossy(&output.stdout);
        if body.is_empty() {
            continue;
        }

        config.reachable = true;

        // Parse the JSON response
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
            // Workers are at: apps.frankenphp.workers[]
            if let Some(workers) = json
                .pointer("/apps/frankenphp/workers")
                .and_then(|v| v.as_array())
            {
                if !workers.is_empty() {
                    config.has_worker = true;
                    // Count worker threads: sum of all workers' "num" values
                    let total: u32 = workers
                        .iter()
                        .filter_map(|w| w.get("num").and_then(|n| n.as_u64()))
                        .map(|n| n as u32)
                        .sum();
                    if total > 0 {
                        config.worker_count = Some(total);
                    }
                }
            }

            // num_threads is at: apps.frankenphp.num_threads
            if let Some(nt) = json
                .pointer("/apps/frankenphp/num_threads")
                .and_then(|v| v.as_u64())
            {
                config.num_threads = Some(nt as u32);
            }
        }

        break; // Found a working admin port
    }

    config
}

/// Parse the Caddyfile and extract configuration relevant to the given app.
/// Matches site blocks by root path containing the app_path.
///
/// Handles multiple site blocks for multi-app servers.
fn parse_site_config(content: &str, app_path: &str) -> SiteConfig {
    let mut config = SiteConfig::default();
    let public_path = format!("{app_path}/public");

    // Track brace depth and whether we're in the matching site block or global block
    let mut in_matching_site = false;
    let mut in_global_block = false;
    let mut brace_depth: i32 = 0;
    let mut global_num_threads: Option<u32> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Count braces on this line
        let opens = trimmed.chars().filter(|&c| c == '{').count() as i32;
        let closes = trimmed.chars().filter(|&c| c == '}').count() as i32;

        // Detect global options block: line starts with { at depth 0
        if brace_depth == 0 && trimmed == "{" {
            in_global_block = true;
            brace_depth += opens - closes;
            continue;
        }

        // Inside global block: look for frankenphp { worker, num_threads }
        if in_global_block {
            if trimmed.starts_with("num_threads") {
                if let Some(val) = trimmed.split_whitespace().nth(1) {
                    global_num_threads = val.parse().ok();
                }
            }

            // Worker directive in global frankenphp block (used by Octane)
            // Short form: worker /path/to/worker.php 8
            // Block form: worker { file ... num ... }
            if trimmed.starts_with("worker") {
                config.has_worker = true;
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 3 {
                    if let Ok(n) = parts[2].parse::<u32>() {
                        config.worker_num = Some(n);
                    }
                }
            }

            // num inside a worker block within the global frankenphp block
            if trimmed.starts_with("num ") {
                if let Some(val) = trimmed.split_whitespace().nth(1) {
                    if let Ok(n) = val.parse::<u32>() {
                        config.worker_num = Some(n);
                    }
                }
            }

            brace_depth += opens - closes;
            if brace_depth <= 0 {
                in_global_block = false;
                brace_depth = 0;
            }
            continue;
        }

        // Detect site block start: a line at depth 0 that opens a block
        if brace_depth == 0 && opens > 0 {
            // Next lines will be inside this site block
            brace_depth += opens - closes;
            in_matching_site = false; // Will be set when we find root
            continue;
        }

        // Inside a site block
        if brace_depth > 0 {
            // Check if root matches our app
            if trimmed.starts_with("root") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if let Some(last) = parts.last() {
                    if last.contains(&public_path) || last.contains(app_path) {
                        in_matching_site = true;
                    }
                }
            }

            // php_server directive
            if trimmed.starts_with("php_server") && in_matching_site {
                config.has_php_server = true;
            }

            // worker directive (inside php_server block)
            if trimmed.starts_with("worker") && in_matching_site {
                config.has_worker = true;
                // Short form: worker index.php 16
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 3 {
                    if let Ok(n) = parts[2].parse::<u32>() {
                        config.worker_num = Some(n);
                    }
                }
            }

            // num or num_threads inside worker block
            if (trimmed.starts_with("num ") || trimmed.starts_with("num_threads"))
                && in_matching_site
            {
                if let Some(val) = trimmed.split_whitespace().nth(1) {
                    if let Ok(n) = val.parse::<u32>() {
                        if trimmed.starts_with("num ") {
                            config.worker_num = Some(n);
                        } else {
                            config.num_threads = Some(n);
                        }
                    }
                }
            }

            brace_depth += opens - closes;
            if brace_depth <= 0 {
                // End of site block — if this was matching, we're done
                if in_matching_site {
                    break;
                }
                brace_depth = 0;
                in_matching_site = false;
            }
            continue;
        }

        brace_depth += opens - closes;
    }

    // Fall back to global num_threads if no site-specific value
    if config.num_threads.is_none() {
        config.num_threads = global_num_threads;
    }

    config
}

/// Check if worker mode is configured for this app.
fn check_worker_mode(
    site_config: &Option<SiteConfig>,
    admin_config: &AdminApiConfig,
    results: &mut Vec<CheckResult>,
) {
    // Prefer admin API (runtime truth) over static Caddyfile parsing
    if admin_config.has_worker {
        let detail = match admin_config.worker_count {
            Some(n) => format!("enabled ({n} workers, via admin API)"),
            None => "enabled (via admin API)".to_string(),
        };
        results.push(CheckResult::ok("FrankenPHP Worker Mode", detail));
        return;
    }

    match site_config {
        Some(cfg) if cfg.has_worker => {
            let detail = match cfg.worker_num {
                Some(n) => format!("enabled ({n} workers)"),
                None => "enabled".to_string(),
            };
            results.push(CheckResult::ok("FrankenPHP Worker Mode", detail));
        }
        Some(cfg) if cfg.has_php_server => {
            results.push(CheckResult::warn(
                "FrankenPHP Worker Mode",
                "php_server without worker — enable worker mode for significantly better performance",
            ));
        }
        _ if admin_config.reachable => {
            // Admin API is up but no workers found — genuinely not using worker mode
            results.push(CheckResult::warn(
                "FrankenPHP Worker Mode",
                "not enabled — enable worker mode for significantly better performance",
            ));
        }
        _ => {
            results.push(CheckResult::warn(
                "FrankenPHP Worker Mode",
                "Could not detect worker configuration (no Caddyfile found, admin API unreachable)",
            ));
        }
    }
}

/// Check if num_threads is explicitly configured.
fn check_num_threads(
    site_config: &Option<SiteConfig>,
    admin_config: &AdminApiConfig,
    ctx: &SystemContext,
    results: &mut Vec<CheckResult>,
) {
    let default_threads = ctx.cpu_cores * 2;

    // Prefer admin API
    if let Some(nt) = admin_config.num_threads {
        results.push(CheckResult::ok(
            "FrankenPHP num_threads",
            format!("{nt} (via admin API)"),
        ));
        return;
    }

    match site_config {
        Some(cfg) => {
            let effective = cfg
                .worker_num
                .or(cfg.num_threads)
                .unwrap_or(default_threads as u32);

            if cfg.worker_num.is_some() || cfg.num_threads.is_some() {
                results.push(CheckResult::ok(
                    "FrankenPHP num_threads",
                    format!("{effective} (explicitly configured)"),
                ));
            } else {
                results.push(CheckResult::warn(
                    "FrankenPHP num_threads",
                    format!(
                        "using default ({default_threads} = 2×{} CPUs) — tune based on load tests",
                        ctx.cpu_cores
                    ),
                ));
            }
        }
        None => {
            results.push(CheckResult::warn(
                "FrankenPHP num_threads",
                format!(
                    "could not read Caddyfile — default is {default_threads} (2×{} CPUs)",
                    ctx.cpu_cores
                ),
            ));
        }
    }
}

/// Check if Caddy log level is set to debug (expensive).
fn check_log_level(caddyfile_content: &str, results: &mut Vec<CheckResult>) {
    let mut found_debug = false;

    for line in caddyfile_content.lines() {
        let trimmed = line.trim();
        // Match: level DEBUG or level debug
        if trimmed.starts_with("level") {
            if let Some(val) = trimmed.split_whitespace().nth(1) {
                if val.eq_ignore_ascii_case("DEBUG") {
                    found_debug = true;
                    break;
                }
            }
        }
    }

    if found_debug {
        results.push(CheckResult::warn(
            "FrankenPHP Log Level",
            "DEBUG — logging at debug level reduces performance (I/O + allocations)",
        ));
    } else {
        results.push(CheckResult::ok("FrankenPHP Log Level", "not set to DEBUG"));
    }
}

/// Check if the document root is a symlink.
/// If it is, FrankenPHP resolves it on every request unless disabled.
fn check_symlink_root(app_path: &str, results: &mut Vec<CheckResult>) {
    let public_path = format!("{app_path}/public");
    let path = Path::new(&public_path);

    match fs::symlink_metadata(&public_path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            results.push(CheckResult::warn(
                "FrankenPHP Symlink Root",
                format!(
                    "{public_path} is a symlink — FrankenPHP resolves it on every request, consider using the real path"
                ),
            ));
        }
        Ok(_) => {
            // Not a symlink — check the app_path itself
            match fs::symlink_metadata(app_path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    results.push(CheckResult::info(
                        "FrankenPHP Symlink Root",
                        format!("{app_path} is a symlink — ensure FrankenPHP root uses the resolved path"),
                    ));
                }
                _ => {
                    results.push(CheckResult::ok(
                        "FrankenPHP Symlink Root",
                        "document root is not a symlink",
                    ));
                }
            }
        }
        Err(_) => {
            // public dir doesn't exist — check if app_path itself is a symlink
            if path.exists() {
                // exists but we can't read metadata — skip
            } else {
                results.push(CheckResult::info(
                    "FrankenPHP Symlink Root",
                    format!("{public_path} not found — skipping symlink check"),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Caddyfile parsing tests ---

    #[test]
    fn parse_single_site_with_worker() {
        let caddyfile = r#"
{
    frankenphp
}

app.example.com {
    root * /home/forge/myapp/public
    php_server {
        worker index.php 16
    }
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(cfg.has_worker);
        assert!(cfg.has_php_server);
        assert_eq!(cfg.worker_num, Some(16));
    }

    #[test]
    fn parse_worker_block_form() {
        let caddyfile = r#"
app.example.com {
    root * /home/forge/myapp/public
    php_server {
        worker {
            file index.php
            num 8
        }
    }
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(cfg.has_worker);
        assert_eq!(cfg.worker_num, Some(8));
    }

    #[test]
    fn parse_multi_site_matches_correct_app() {
        let caddyfile = r#"
{
    frankenphp
}

one.example.com {
    root * /home/forge/app-one/public
    php_server {
        worker index.php 4
    }
}

two.example.com {
    root * /home/forge/app-two/public
    php_server {
        worker index.php 12
    }
}
"#;
        let cfg1 = parse_site_config(caddyfile, "/home/forge/app-one");
        assert_eq!(cfg1.worker_num, Some(4));

        let cfg2 = parse_site_config(caddyfile, "/home/forge/app-two");
        assert_eq!(cfg2.worker_num, Some(12));
    }

    #[test]
    fn parse_no_worker_mode() {
        let caddyfile = r#"
app.example.com {
    root * /home/forge/myapp/public
    php_server
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(!cfg.has_worker);
        assert!(cfg.has_php_server);
    }

    #[test]
    fn parse_global_num_threads() {
        let caddyfile = r#"
{
    frankenphp {
        num_threads 32
    }
}

app.example.com {
    root * /home/forge/myapp/public
    php_server {
        worker index.php
    }
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert_eq!(cfg.num_threads, Some(32));
    }

    #[test]
    fn parse_global_worker_short_form() {
        let caddyfile = r#"
{
    frankenphp {
        worker /home/forge/myapp/public/frankenphp-worker.php 8
    }
}

:443 {
    root * /home/forge/myapp/public
    php_server
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(cfg.has_worker);
        assert_eq!(cfg.worker_num, Some(8));
    }

    #[test]
    fn parse_global_worker_block_form() {
        let caddyfile = r#"
{
    frankenphp {
        worker {
            file /home/forge/myapp/public/frankenphp-worker.php
            num 12
        }
    }
}

:443 {
    root * /home/forge/myapp/public
    php_server
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(cfg.has_worker);
        assert_eq!(cfg.worker_num, Some(12));
    }

    #[test]
    fn parse_octane_stub_style() {
        // Simulates the Octane-generated Caddyfile with env vars resolved
        let caddyfile = r#"
{
    admin localhost:2019

    frankenphp {
        worker {
            file "/home/forge/myapp/public/frankenphp-worker.php"
            num 4
        }
    }
}

:443 {
    log {
        level WARN
    }

    route {
        root * "/home/forge/myapp/public"
        encode zstd br gzip

        php_server {
            index frankenphp-worker.php
            try_files {path} frankenphp-worker.php
            resolve_root_symlink
        }
    }
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(cfg.has_worker, "should detect worker in global block");
        assert_eq!(cfg.worker_num, Some(4));
        assert!(cfg.has_php_server);
    }

    #[test]
    fn parse_no_matching_site() {
        let caddyfile = r#"
other.example.com {
    root * /home/forge/other-app/public
    php_server {
        worker index.php 8
    }
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(!cfg.has_worker);
        assert!(!cfg.has_php_server);
    }

    #[test]
    fn parse_empty_caddyfile() {
        let cfg = parse_site_config("", "/home/forge/myapp");
        assert!(!cfg.has_worker);
        assert!(!cfg.has_php_server);
        assert_eq!(cfg.num_threads, None);
    }

    #[test]
    fn parse_comments_and_blank_lines() {
        let caddyfile = r#"
# This is a comment

app.example.com {
    # root is set below
    root * /home/forge/myapp/public
    php_server {
        worker index.php 6
    }
}
"#;
        let cfg = parse_site_config(caddyfile, "/home/forge/myapp");
        assert!(cfg.has_worker);
        assert_eq!(cfg.worker_num, Some(6));
    }

    // --- Worker mode check tests ---

    fn no_admin() -> AdminApiConfig {
        AdminApiConfig::default()
    }

    #[test]
    fn worker_mode_ok_when_present() {
        let cfg = SiteConfig {
            has_worker: true,
            worker_num: Some(8),
            has_php_server: true,
            ..Default::default()
        };
        let mut results = Vec::new();
        check_worker_mode(&Some(cfg), &no_admin(), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
        assert!(results[0].detail.contains("8 workers"));
    }

    #[test]
    fn worker_mode_ok_from_admin_api() {
        let admin = AdminApiConfig {
            has_worker: true,
            worker_count: Some(4),
            reachable: true,
            ..Default::default()
        };
        let mut results = Vec::new();
        check_worker_mode(&None, &admin, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
        assert!(results[0].detail.contains("4 workers"));
        assert!(results[0].detail.contains("admin API"));
    }

    #[test]
    fn worker_mode_admin_api_overrides_caddyfile() {
        let cfg = SiteConfig {
            has_php_server: true,
            has_worker: false,
            ..Default::default()
        };
        let admin = AdminApiConfig {
            has_worker: true,
            worker_count: Some(8),
            reachable: true,
            ..Default::default()
        };
        let mut results = Vec::new();
        check_worker_mode(&Some(cfg), &admin, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
    }

    #[test]
    fn worker_mode_warn_when_php_server_only() {
        let cfg = SiteConfig {
            has_worker: false,
            has_php_server: true,
            ..Default::default()
        };
        let mut results = Vec::new();
        check_worker_mode(&Some(cfg), &no_admin(), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
    }

    #[test]
    fn worker_mode_warn_when_no_config() {
        let mut results = Vec::new();
        check_worker_mode(&None, &no_admin(), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
    }

    #[test]
    fn worker_mode_warn_admin_reachable_no_workers() {
        let admin = AdminApiConfig {
            reachable: true,
            has_worker: false,
            ..Default::default()
        };
        let mut results = Vec::new();
        check_worker_mode(&None, &admin, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
        assert!(results[0].detail.contains("not enabled"));
    }

    // --- num_threads check tests ---

    #[test]
    fn num_threads_ok_when_explicit() {
        let cfg = SiteConfig {
            num_threads: Some(16),
            ..Default::default()
        };
        let ctx = dummy_ctx(4);
        let mut results = Vec::new();
        check_num_threads(&Some(cfg), &no_admin(), &ctx, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
        assert!(results[0].detail.contains("16"));
    }

    #[test]
    fn num_threads_ok_from_admin_api() {
        let admin = AdminApiConfig {
            num_threads: Some(32),
            reachable: true,
            ..Default::default()
        };
        let ctx = dummy_ctx(4);
        let mut results = Vec::new();
        check_num_threads(&None, &admin, &ctx, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
        assert!(results[0].detail.contains("32"));
    }

    #[test]
    fn num_threads_warn_when_default() {
        let cfg = SiteConfig::default();
        let ctx = dummy_ctx(4);
        let mut results = Vec::new();
        check_num_threads(&Some(cfg), &no_admin(), &ctx, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
        assert!(results[0].detail.contains("8")); // 2×4
    }

    #[test]
    fn num_threads_uses_worker_num_as_fallback() {
        let cfg = SiteConfig {
            has_worker: true,
            worker_num: Some(12),
            num_threads: None,
            ..Default::default()
        };
        let ctx = dummy_ctx(4);
        let mut results = Vec::new();
        check_num_threads(&Some(cfg), &no_admin(), &ctx, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
        assert!(results[0].detail.contains("12"));
    }

    // --- Log level check tests ---

    #[test]
    fn log_level_ok_when_not_debug() {
        let mut results = Vec::new();
        check_log_level("level INFO\n", &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
    }

    #[test]
    fn log_level_warn_when_debug() {
        let mut results = Vec::new();
        check_log_level("log {\n    level DEBUG\n}\n", &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
    }

    #[test]
    fn log_level_case_insensitive() {
        let mut results = Vec::new();
        check_log_level("level debug\n", &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
    }

    #[test]
    fn log_level_ok_no_level_directive() {
        let mut results = Vec::new();
        check_log_level("php_server\n", &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
    }

    // --- Symlink check tests ---

    #[test]
    fn symlink_check_non_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        std::fs::create_dir(&public).unwrap();

        let mut results = Vec::new();
        check_symlink_root(dir.path().to_str().unwrap(), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
    }

    #[test]
    fn symlink_check_detects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real_public = dir.path().join("real_public");
        std::fs::create_dir(&real_public).unwrap();
        let link_public = dir.path().join("public");
        std::os::unix::fs::symlink(&real_public, &link_public).unwrap();

        let mut results = Vec::new();
        check_symlink_root(dir.path().to_str().unwrap(), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
    }

    #[test]
    fn symlink_check_missing_public() {
        let dir = tempfile::tempdir().unwrap();
        let mut results = Vec::new();
        check_symlink_root(dir.path().to_str().unwrap(), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Info);
    }

    // --- Helper ---

    fn dummy_ctx(cpu_cores: usize) -> SystemContext {
        SystemContext {
            cpu_cores,
            total_ram_mb: 4096,
            available_ram_mb: 2048,
            swap_used_mb: 0,
            mysql_running: false,
            mysql_pid: None,
            mysql_ram_mb: 0,
            redis_running: false,
            redis_pid: None,
            redis_ram_mb: 0,
            php_ram_budget_mb: 2048,
            laravel_version: None,
            laravel_major: None,
        }
    }
}

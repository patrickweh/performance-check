use crate::types::{CheckResult, SystemContext};
use std::fs;
use std::path::Path;
use std::process::Command;

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
    let caddyfile = find_caddyfile();
    let site_config = caddyfile
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|content| parse_site_config(&content, app_path));

    // Worker Mode check
    check_worker_mode(&site_config, &mut results);

    // num_threads tuning check
    check_num_threads(&site_config, ctx, &mut results);

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
fn find_caddyfile() -> Option<String> {
    let paths = ["/etc/caddy/Caddyfile", "/etc/frankenphp/Caddyfile"];
    for path in &paths {
        if Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    None
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

        // Inside global block: look for frankenphp { num_threads }
        if in_global_block {
            if trimmed.starts_with("num_threads") {
                if let Some(val) = trimmed.split_whitespace().nth(1) {
                    global_num_threads = val.parse().ok();
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
fn check_worker_mode(site_config: &Option<SiteConfig>, results: &mut Vec<CheckResult>) {
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
        _ => {
            results.push(CheckResult::warn(
                "FrankenPHP Worker Mode",
                "Could not detect worker configuration in Caddyfile",
            ));
        }
    }
}

/// Check if num_threads is explicitly configured.
fn check_num_threads(
    site_config: &Option<SiteConfig>,
    ctx: &SystemContext,
    results: &mut Vec<CheckResult>,
) {
    let default_threads = ctx.cpu_cores * 2;

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

    #[test]
    fn worker_mode_ok_when_present() {
        let cfg = SiteConfig {
            has_worker: true,
            worker_num: Some(8),
            has_php_server: true,
            ..Default::default()
        };
        let mut results = Vec::new();
        check_worker_mode(&Some(cfg), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
        assert!(results[0].detail.contains("8 workers"));
    }

    #[test]
    fn worker_mode_warn_when_php_server_only() {
        let cfg = SiteConfig {
            has_worker: false,
            has_php_server: true,
            ..Default::default()
        };
        let mut results = Vec::new();
        check_worker_mode(&Some(cfg), &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
    }

    #[test]
    fn worker_mode_warn_when_no_config() {
        let mut results = Vec::new();
        check_worker_mode(&None, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Warn);
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
        check_num_threads(&Some(cfg), &ctx, &mut results);
        assert_eq!(results[0].status, crate::types::Status::Ok);
        assert!(results[0].detail.contains("16"));
    }

    #[test]
    fn num_threads_warn_when_default() {
        let cfg = SiteConfig::default();
        let ctx = dummy_ctx(4);
        let mut results = Vec::new();
        check_num_threads(&Some(cfg), &ctx, &mut results);
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
        check_num_threads(&Some(cfg), &ctx, &mut results);
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

use crate::types::{CheckResult, SystemContext};
use std::fs;
use std::path::Path;

pub fn check(app_path: &str, ctx: &SystemContext) -> Vec<CheckResult> {
    let mut results = Vec::new();

    check_env(app_path, ctx, &mut results);
    check_bootstrap_cache(app_path, ctx, &mut results);
    check_composer(app_path, &mut results);

    results
}

fn check_env(app_path: &str, ctx: &SystemContext, results: &mut Vec<CheckResult>) {
    let env_path = Path::new(app_path).join(".env");
    let env_file = &env_path.to_string_lossy().to_string();

    let content = match fs::read_to_string(&env_path) {
        Ok(c) => c,
        Err(_) => {
            results.push(CheckResult::fail("Laravel .env", "File not found"));
            return;
        }
    };

    let get = |key: &str| -> Option<String> {
        content
            .lines()
            .find(|l| l.starts_with(&format!("{key}=")))
            .map(|l| {
                l.split_once('=')
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim()
                    .to_string()
            })
    };

    // APP_ENV
    match get("APP_ENV").as_deref() {
        Some("production") => results.push(CheckResult::ok("APP_ENV", "production")),
        Some(v) => results.push(
            CheckResult::fail("APP_ENV", format!("'{v}' — should be 'production'")).with_fix(
                "Set APP_ENV=production",
                env_file,
                "APP_ENV=production",
            ),
        ),
        None => results.push(CheckResult::fail("APP_ENV", "Not set")),
    }

    // APP_DEBUG
    match get("APP_DEBUG").as_deref() {
        Some("false") => results.push(CheckResult::ok("APP_DEBUG", "false")),
        Some(v) => results.push(
            CheckResult::fail("APP_DEBUG", format!("'{v}' — MUST be false in production"))
                .with_fix("Set APP_DEBUG=false", env_file, "APP_DEBUG=false"),
        ),
        None => results.push(CheckResult::warn(
            "APP_DEBUG",
            "Not set — defaults to false",
        )),
    }

    // OCTANE_HTTPS
    match get("OCTANE_HTTPS").as_deref() {
        Some("true") => results.push(CheckResult::ok("OCTANE_HTTPS", "true")),
        _ => results.push(
            CheckResult::warn(
                "OCTANE_HTTPS",
                "Not set or not 'true' — URL generation may break under HTTPS",
            )
            .with_fix("Set OCTANE_HTTPS=true", env_file, "OCTANE_HTTPS=true"),
        ),
    }

    // CACHE_STORE
    match get("CACHE_STORE").as_deref() {
        Some("redis") => results.push(CheckResult::ok("CACHE_STORE", "redis")),
        Some(v) if ctx.redis_running => results.push(
            CheckResult::warn(
                "CACHE_STORE",
                format!("'{v}' — Redis is running, consider using redis"),
            )
            .with_fix("Set CACHE_STORE=redis", env_file, "CACHE_STORE=redis"),
        ),
        Some(v) => results.push(CheckResult::info("CACHE_STORE", v.to_string())),
        None => {
            if ctx.redis_running {
                results.push(
                    CheckResult::warn("CACHE_STORE", "Not set — Redis is running, recommend redis")
                        .with_fix("Set CACHE_STORE=redis", env_file, "CACHE_STORE=redis"),
                );
            } else {
                results.push(CheckResult::info("CACHE_STORE", "Not set"));
            }
        }
    }

    // QUEUE_CONNECTION
    match get("QUEUE_CONNECTION").as_deref() {
        Some("sync") => results.push(
            CheckResult::fail(
                "QUEUE_CONNECTION",
                "'sync' — queued jobs run inline, blocking requests",
            )
            .with_fix(
                "Set QUEUE_CONNECTION=redis",
                env_file,
                "QUEUE_CONNECTION=redis",
            ),
        ),
        Some(v) => results.push(CheckResult::ok("QUEUE_CONNECTION", v.to_string())),
        None => results.push(CheckResult::warn("QUEUE_CONNECTION", "Not set")),
    }

    // SESSION_DRIVER
    match get("SESSION_DRIVER").as_deref() {
        Some("redis") => results.push(CheckResult::ok("SESSION_DRIVER", "redis")),
        Some("file") if ctx.redis_running => results.push(
            CheckResult::warn(
                "SESSION_DRIVER",
                "'file' — Redis is running, file sessions don't scale with Octane workers",
            )
            .with_fix("Set SESSION_DRIVER=redis", env_file, "SESSION_DRIVER=redis"),
        ),
        Some(v) => results.push(CheckResult::ok("SESSION_DRIVER", v.to_string())),
        None => results.push(CheckResult::warn("SESSION_DRIVER", "Not set")),
    }

    // LOG_CHANNEL
    match get("LOG_CHANNEL").as_deref() {
        Some("stderr") | Some("syslog") | Some("errorlog") | Some("flare") => {
            results.push(CheckResult::ok("LOG_CHANNEL", get("LOG_CHANNEL").unwrap()));
        }
        Some("single") | Some("daily") => {
            results.push(
                CheckResult::warn(
                    "LOG_CHANNEL",
                    format!("'{}' — file-based logging problematic with Octane (open file handles, rotation issues). Use 'stderr'", get("LOG_CHANNEL").unwrap()),
                )
                .with_fix("Set LOG_CHANNEL=stderr", env_file, "LOG_CHANNEL=stderr"),
            );
        }
        Some("stack") => {
            results.push(
                CheckResult::warn(
                    "LOG_CHANNEL",
                    "'stack' — may include file-based channels (single/daily) which are problematic with Octane. Check config/logging.php or use 'stderr'",
                )
                .with_fix("Set LOG_CHANNEL=stderr", env_file, "LOG_CHANNEL=stderr"),
            );
        }
        Some(v) => results.push(CheckResult::info("LOG_CHANNEL", v.to_string())),
        None => results.push(CheckResult::info("LOG_CHANNEL", "Not set (using default)")),
    }
}

fn check_bootstrap_cache(app_path: &str, ctx: &SystemContext, results: &mut Vec<CheckResult>) {
    let cache_dir = Path::new(app_path).join("bootstrap/cache");

    let mut expected: Vec<&str> = vec!["config.php", "routes-v7.php", "services.php"];

    let is_laravel_11_plus = ctx.laravel_major.is_some_and(|v| v >= 11);
    if !is_laravel_11_plus {
        expected.push("packages.php");
    }

    for file in &expected {
        let path = cache_dir.join(file);
        if path.exists() {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size > 0 {
                results.push(CheckResult::ok(
                    format!("Bootstrap cache: {file}"),
                    "exists",
                ));
            } else {
                results.push(CheckResult::warn(
                    format!("Bootstrap cache: {file}"),
                    "exists but empty — run php artisan optimize",
                ));
            }
        } else {
            results.push(CheckResult::warn(
                format!("Bootstrap cache: {file}"),
                "missing — run php artisan optimize",
            ));
        }
    }
}

fn check_composer(app_path: &str, results: &mut Vec<CheckResult>) {
    let classmap_path = Path::new(app_path).join("vendor/composer/autoload_classmap.php");

    if !classmap_path.exists() {
        results.push(CheckResult::fail(
            "Composer Classmap",
            "vendor/composer/autoload_classmap.php not found — run composer install",
        ));
        return;
    }

    let content = fs::read_to_string(&classmap_path).unwrap_or_default();
    let entries = content.matches("=>").count();

    if entries < 100 {
        results.push(CheckResult::warn(
            "Composer Classmap",
            format!(
                "Only {entries} entries — run composer dump-autoload -o for optimized autoloader"
            ),
        ));
    } else {
        results.push(CheckResult::ok(
            "Composer Classmap",
            format!("{entries} entries (optimized)"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Status, SystemContext};
    use tempfile;

    fn test_ctx(redis_running: bool, laravel_major: Option<u32>) -> SystemContext {
        SystemContext {
            cpu_cores: 4,
            total_ram_mb: 4096,
            available_ram_mb: 2048,
            swap_used_mb: 0,
            mysql_running: false,
            mysql_pid: None,
            mysql_ram_mb: 0,
            redis_running,
            redis_pid: None,
            redis_ram_mb: 0,
            php_ram_budget_mb: 3072,
            laravel_version: laravel_major.map(|v| format!("{v}.0.0")),
            laravel_major,
        }
    }

    fn create_laravel_app(dir: &std::path::Path, env_content: &str) {
        fs::write(dir.join(".env"), env_content).unwrap();
    }

    // --- .env parsing tests ---

    #[test]
    fn env_production_all_good() {
        let dir = tempfile::tempdir().unwrap();
        create_laravel_app(
            dir.path(),
            "\
APP_ENV=production
APP_DEBUG=false
OCTANE_HTTPS=true
CACHE_STORE=redis
QUEUE_CONNECTION=redis
SESSION_DRIVER=redis
LOG_CHANNEL=stderr
",
        );
        let ctx = test_ctx(true, Some(11));
        let mut results = Vec::new();
        check_env(dir.path().to_str().unwrap(), &ctx, &mut results);

        let ok_count = results.iter().filter(|r| r.status == Status::Ok).count();
        let fail_count = results.iter().filter(|r| r.status == Status::Fail).count();
        assert_eq!(fail_count, 0, "Production config should have no failures");
        assert!(
            ok_count >= 5,
            "Expected at least 5 OK results, got {ok_count}"
        );
    }

    #[test]
    fn env_debug_true_is_fail() {
        let dir = tempfile::tempdir().unwrap();
        create_laravel_app(dir.path(), "APP_ENV=production\nAPP_DEBUG=true\n");
        let ctx = test_ctx(false, None);
        let mut results = Vec::new();
        check_env(dir.path().to_str().unwrap(), &ctx, &mut results);

        let debug_result = results.iter().find(|r| r.label == "APP_DEBUG").unwrap();
        assert_eq!(debug_result.status, Status::Fail);
        assert!(debug_result.fix.is_some());
    }

    #[test]
    fn env_sync_queue_is_fail() {
        let dir = tempfile::tempdir().unwrap();
        create_laravel_app(dir.path(), "QUEUE_CONNECTION=sync\n");
        let ctx = test_ctx(false, None);
        let mut results = Vec::new();
        check_env(dir.path().to_str().unwrap(), &ctx, &mut results);

        let queue_result = results
            .iter()
            .find(|r| r.label == "QUEUE_CONNECTION")
            .unwrap();
        assert_eq!(queue_result.status, Status::Fail);
    }

    #[test]
    fn env_file_session_with_redis_is_warn() {
        let dir = tempfile::tempdir().unwrap();
        create_laravel_app(dir.path(), "SESSION_DRIVER=file\n");
        let ctx = test_ctx(true, None); // Redis running
        let mut results = Vec::new();
        check_env(dir.path().to_str().unwrap(), &ctx, &mut results);

        let session_result = results
            .iter()
            .find(|r| r.label == "SESSION_DRIVER")
            .unwrap();
        assert_eq!(session_result.status, Status::Warn);
    }

    #[test]
    fn env_file_session_without_redis_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        create_laravel_app(dir.path(), "SESSION_DRIVER=file\n");
        let ctx = test_ctx(false, None); // Redis NOT running
        let mut results = Vec::new();
        check_env(dir.path().to_str().unwrap(), &ctx, &mut results);

        let session_result = results
            .iter()
            .find(|r| r.label == "SESSION_DRIVER")
            .unwrap();
        assert_eq!(session_result.status, Status::Ok);
    }

    #[test]
    fn env_daily_log_is_warn() {
        let dir = tempfile::tempdir().unwrap();
        create_laravel_app(dir.path(), "LOG_CHANNEL=daily\n");
        let ctx = test_ctx(false, None);
        let mut results = Vec::new();
        check_env(dir.path().to_str().unwrap(), &ctx, &mut results);

        let log_result = results.iter().find(|r| r.label == "LOG_CHANNEL").unwrap();
        assert_eq!(log_result.status, Status::Warn);
    }

    #[test]
    fn env_missing_file_is_fail() {
        let dir = tempfile::tempdir().unwrap();
        // Don't create .env
        let ctx = test_ctx(false, None);
        let mut results = Vec::new();
        check_env(dir.path().to_str().unwrap(), &ctx, &mut results);

        assert_eq!(results[0].status, Status::Fail);
        assert!(results[0].label.contains(".env"));
    }

    // --- Bootstrap cache tests ---

    #[test]
    fn bootstrap_cache_all_present() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("bootstrap/cache");
        fs::create_dir_all(&cache_dir).unwrap();

        for file in &["config.php", "routes-v7.php", "services.php"] {
            fs::write(cache_dir.join(file), "<?php return [];").unwrap();
        }

        let ctx = test_ctx(false, Some(11));
        let mut results = Vec::new();
        check_bootstrap_cache(dir.path().to_str().unwrap(), &ctx, &mut results);

        assert!(results.iter().all(|r| r.status == Status::Ok));
        assert_eq!(results.len(), 3); // Laravel 11+ doesn't check packages.php
    }

    #[test]
    fn bootstrap_cache_laravel_10_checks_packages() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("bootstrap/cache");
        fs::create_dir_all(&cache_dir).unwrap();

        let ctx = test_ctx(false, Some(10));
        let mut results = Vec::new();
        check_bootstrap_cache(dir.path().to_str().unwrap(), &ctx, &mut results);

        // Laravel 10 should check 4 files including packages.php
        assert_eq!(results.len(), 4);
        assert!(results.iter().any(|r| r.label.contains("packages.php")));
    }

    #[test]
    fn bootstrap_cache_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(false, Some(11));
        let mut results = Vec::new();
        check_bootstrap_cache(dir.path().to_str().unwrap(), &ctx, &mut results);

        assert!(results.iter().all(|r| r.status == Status::Warn));
    }

    #[test]
    fn bootstrap_cache_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("bootstrap/cache");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("config.php"), "").unwrap();

        let ctx = test_ctx(false, Some(11));
        let mut results = Vec::new();
        check_bootstrap_cache(dir.path().to_str().unwrap(), &ctx, &mut results);

        let config_result = results
            .iter()
            .find(|r| r.label.contains("config.php"))
            .unwrap();
        assert_eq!(config_result.status, Status::Warn);
        assert!(config_result.detail.contains("empty"));
    }

    // --- Composer tests ---

    #[test]
    fn composer_classmap_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut results = Vec::new();
        check_composer(dir.path().to_str().unwrap(), &mut results);

        assert_eq!(results[0].status, Status::Fail);
    }

    #[test]
    fn composer_classmap_few_entries() {
        let dir = tempfile::tempdir().unwrap();
        let classmap_dir = dir.path().join("vendor/composer");
        fs::create_dir_all(&classmap_dir).unwrap();

        // Create classmap with only 5 entries
        let content: String = (0..5)
            .map(|i| format!("'Class{i}' => '/path/{i}.php',\n"))
            .collect();
        fs::write(
            classmap_dir.join("autoload_classmap.php"),
            format!("<?php\nreturn array(\n{content});"),
        )
        .unwrap();

        let mut results = Vec::new();
        check_composer(dir.path().to_str().unwrap(), &mut results);

        assert_eq!(results[0].status, Status::Warn);
        assert!(results[0].detail.contains("dump-autoload"));
    }

    #[test]
    fn composer_classmap_optimized() {
        let dir = tempfile::tempdir().unwrap();
        let classmap_dir = dir.path().join("vendor/composer");
        fs::create_dir_all(&classmap_dir).unwrap();

        // Create classmap with 150 entries
        let content: String = (0..150)
            .map(|i| format!("'Class{i}' => '/path/{i}.php',\n"))
            .collect();
        fs::write(
            classmap_dir.join("autoload_classmap.php"),
            format!("<?php\nreturn array(\n{content});"),
        )
        .unwrap();

        let mut results = Vec::new();
        check_composer(dir.path().to_str().unwrap(), &mut results);

        assert_eq!(results[0].status, Status::Ok);
        assert!(results[0].detail.contains("optimized"));
    }
}

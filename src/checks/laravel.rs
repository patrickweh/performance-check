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
            .map(|l| l.splitn(2, '=').nth(1).unwrap_or("").trim().to_string())
    };

    // APP_ENV
    match get("APP_ENV").as_deref() {
        Some("production") => results.push(CheckResult::ok("APP_ENV", "production")),
        Some(v) => results.push(
            CheckResult::fail("APP_ENV", format!("'{v}' — should be 'production'"))
                .with_fix("Set APP_ENV=production", env_file, "APP_ENV=production"),
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
        None => results.push(CheckResult::warn("APP_DEBUG", "Not set — defaults to false")),
    }

    // OCTANE_HTTPS
    match get("OCTANE_HTTPS").as_deref() {
        Some("true") => results.push(CheckResult::ok("OCTANE_HTTPS", "true")),
        _ => results.push(
            CheckResult::warn("OCTANE_HTTPS", "Not set or not 'true' — URL generation may break under HTTPS")
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
            .with_fix("Set QUEUE_CONNECTION=redis", env_file, "QUEUE_CONNECTION=redis"),
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
        Some("stderr") | Some("syslog") => {
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

    let is_laravel_11_plus = ctx.laravel_major.map_or(false, |v| v >= 11);
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
            format!("Only {entries} entries — run composer dump-autoload -o for optimized autoloader"),
        ));
    } else {
        results.push(CheckResult::ok(
            "Composer Classmap",
            format!("{entries} entries (optimized)"),
        ));
    }
}

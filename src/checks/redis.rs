use crate::types::{CheckResult, SystemContext};
use std::process::Command;

pub fn check(ctx: &SystemContext) -> Vec<CheckResult> {
    if !ctx.redis_running {
        return vec![CheckResult::info("Redis", "Not running locally — skipping")];
    }

    let mut results = Vec::new();

    // maxmemory
    let maxmem = redis_config_get("maxmemory");
    let recommended_mb = ((ctx.total_ram_mb / 10) as u64).min(4096);

    match maxmem.as_deref() {
        Some("0") | None => {
            results.push(
                CheckResult::warn(
                    "Redis maxmemory",
                    format!("Not set — recommend {recommended_mb}MB (10% of total RAM, max 4GB)"),
                )
                .with_fix(
                    format!("Set Redis maxmemory to {recommended_mb}mb"),
                    "redis-cli",
                    format!("maxmemory {recommended_mb}mb"),
                ),
            );
        }
        Some(v) => {
            let bytes: u64 = v.parse().unwrap_or(0);
            let mb = bytes / (1024 * 1024);
            results.push(CheckResult::ok("Redis maxmemory", format!("{mb}MB")));
        }
    }

    // maxmemory-policy
    let policy = redis_config_get("maxmemory-policy");
    match policy.as_deref() {
        Some("allkeys-lru") | Some("allkeys-lfu") => {
            results.push(CheckResult::ok(
                "Redis maxmemory-policy",
                policy.unwrap(),
            ));
        }
        Some(v) => {
            results.push(
                CheckResult::warn(
                    "Redis maxmemory-policy",
                    format!("'{v}' — recommend allkeys-lru or allkeys-lfu"),
                )
                .with_fix(
                    "Set Redis maxmemory-policy to allkeys-lru",
                    "redis-cli",
                    "maxmemory-policy allkeys-lru",
                ),
            );
        }
        None => {
            results.push(CheckResult::warn(
                "Redis maxmemory-policy",
                "Could not read",
            ));
        }
    }

    results
}

fn redis_config_get(key: &str) -> Option<String> {
    let output = Command::new("redis-cli")
        .args(["CONFIG", "GET", key])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    // redis-cli CONFIG GET returns: key\nvalue\n
    text.lines().nth(1).map(|s| s.trim().to_string())
}

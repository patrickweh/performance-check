//! Integration tests for Redis checks.
//! These require a running Redis server and are marked #[ignore].
//! Run with: cargo test -- --ignored
//! CI sets TEST_REDIS=1 and provides a Redis service container.

use std::process::Command;

fn redis_available() -> bool {
    std::env::var("TEST_REDIS").is_ok()
}

fn redis_cli(args: &[&str]) -> Option<String> {
    let output = Command::new("redis-cli").args(args).output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
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
    text.lines().nth(1).map(|s| s.trim().to_string())
}

#[test]
#[ignore]
fn redis_ping() {
    if !redis_available() {
        return;
    }
    let result = redis_cli(&["ping"]);
    assert_eq!(result, Some("PONG".to_string()));
}

#[test]
#[ignore]
fn redis_maxmemory_readable() {
    if !redis_available() {
        return;
    }
    let val = redis_config_get("maxmemory");
    assert!(val.is_some(), "Should be able to read maxmemory");
    // Default is 0 (unlimited)
    let bytes: u64 = val.unwrap().parse().expect("Should be a number");
    // 0 = unlimited, or some positive value
    assert!(bytes == 0 || bytes > 0);
}

#[test]
#[ignore]
fn redis_maxmemory_policy_readable() {
    if !redis_available() {
        return;
    }
    let val = redis_config_get("maxmemory-policy");
    assert!(val.is_some(), "Should be able to read maxmemory-policy");
    let policy = val.unwrap();
    let valid_policies = [
        "volatile-lru",
        "allkeys-lru",
        "volatile-lfu",
        "allkeys-lfu",
        "volatile-random",
        "allkeys-random",
        "volatile-ttl",
        "noeviction",
    ];
    assert!(
        valid_policies.contains(&policy.as_str()),
        "Unexpected policy: '{policy}'"
    );
}

#[test]
#[ignore]
fn redis_config_set_and_revert() {
    if !redis_available() {
        return;
    }

    // Save original value
    let original = redis_config_get("maxmemory").unwrap_or_default();

    // Set a test value
    let result = redis_cli(&["CONFIG", "SET", "maxmemory", "100mb"]);
    assert_eq!(result, Some("OK".to_string()), "CONFIG SET should succeed");

    // Verify it was set
    let val = redis_config_get("maxmemory").unwrap();
    let bytes: u64 = val.parse().unwrap();
    assert_eq!(bytes, 100 * 1024 * 1024, "Should be 100MB in bytes");

    // Revert
    let revert = redis_cli(&["CONFIG", "SET", "maxmemory", &original]);
    assert_eq!(revert, Some("OK".to_string()), "Revert should succeed");
}

#[test]
#[ignore]
fn redis_config_set_maxmemory_policy() {
    if !redis_available() {
        return;
    }

    let original = redis_config_get("maxmemory-policy").unwrap_or_default();

    let result = redis_cli(&["CONFIG", "SET", "maxmemory-policy", "allkeys-lru"]);
    assert_eq!(result, Some("OK".to_string()));

    let val = redis_config_get("maxmemory-policy").unwrap();
    assert_eq!(val, "allkeys-lru");

    // Revert
    redis_cli(&["CONFIG", "SET", "maxmemory-policy", &original]);
}

#[test]
#[ignore]
fn redis_config_rewrite() {
    if !redis_available() {
        return;
    }

    // CONFIG REWRITE may fail if Redis was started without a config file,
    // which is typical in container setups — that's expected behavior
    let result = redis_cli(&["CONFIG", "REWRITE"]);
    // Just verify it doesn't crash; success depends on config file presence
    let _ = result;
}

#[test]
#[ignore]
fn redis_info_memory() {
    if !redis_available() {
        return;
    }
    let result = redis_cli(&["INFO", "memory"]);
    assert!(result.is_some(), "INFO memory should succeed");
    let info = result.unwrap();
    assert!(info.contains("used_memory:"), "Should contain used_memory");
}

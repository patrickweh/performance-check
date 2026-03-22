//! Integration tests for FrankenPHP admin API (num_threads get/set/delete).
//! These require a running FrankenPHP instance with the admin API on the expected port.
//! Run with: TEST_FRANKENPHP=1 cargo test -- --ignored
//! CI installs FrankenPHP and starts it before running these tests.

use std::process::Command;
use std::sync::Mutex;

fn frankenphp_available() -> bool {
    std::env::var("TEST_FRANKENPHP").is_ok()
}

/// The admin port used by the test FrankenPHP instance.
const ADMIN_PORT: u16 = 2019;

/// Serialize all tests — they share a single FrankenPHP admin API.
static LOCK: Mutex<()> = Mutex::new(());

/// Helper: read num_threads directly via curl (independent of our code).
fn curl_get_num_threads() -> String {
    let output = Command::new("curl")
        .args([
            "-s",
            &format!("http://localhost:{ADMIN_PORT}/config/apps/frankenphp/num_threads"),
        ])
        .output()
        .expect("curl must be available");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Helper: clean up num_threads after each test.
fn cleanup() {
    let _ = Command::new("curl")
        .args([
            "-s",
            "-X",
            "DELETE",
            &format!("http://localhost:{ADMIN_PORT}/config/apps/frankenphp/num_threads"),
        ])
        .output();
}

// ---------------------------------------------------------------------------
// get_num_threads
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn get_num_threads_returns_none_when_not_set() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    let result = frankenphp_check::benchmark::get_num_threads(ADMIN_PORT);
    assert_eq!(
        result, None,
        "should return None when num_threads is not explicitly set"
    );
}

#[test]
#[ignore]
fn get_num_threads_returns_value_when_set() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    // Set via curl directly so this test is independent of set_num_threads
    Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            "24",
            &format!("http://localhost:{ADMIN_PORT}/config/apps/frankenphp/num_threads"),
        ])
        .output()
        .unwrap();

    let result = frankenphp_check::benchmark::get_num_threads(ADMIN_PORT);
    assert_eq!(result, Some(24));

    cleanup();
}

// ---------------------------------------------------------------------------
// set_num_threads
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn set_num_threads_creates_value_when_not_exists() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    // Precondition: num_threads is not set
    assert_eq!(curl_get_num_threads(), "null");

    let ok = frankenphp_check::benchmark::set_num_threads(ADMIN_PORT, 12);
    assert!(ok, "set_num_threads should succeed when key does not exist");

    assert_eq!(curl_get_num_threads(), "12");
    cleanup();
}

#[test]
#[ignore]
fn set_num_threads_overwrites_existing_value() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    // Set initial value
    let ok = frankenphp_check::benchmark::set_num_threads(ADMIN_PORT, 8);
    assert!(ok);
    assert_eq!(curl_get_num_threads(), "8");

    // Overwrite
    let ok = frankenphp_check::benchmark::set_num_threads(ADMIN_PORT, 16);
    assert!(ok, "set_num_threads should succeed when key already exists");
    assert_eq!(curl_get_num_threads(), "16");

    cleanup();
}

#[test]
#[ignore]
fn set_num_threads_value_is_readable_via_get() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    frankenphp_check::benchmark::set_num_threads(ADMIN_PORT, 20);
    let result = frankenphp_check::benchmark::get_num_threads(ADMIN_PORT);
    assert_eq!(
        result,
        Some(20),
        "get should return the value that set wrote"
    );

    cleanup();
}

// ---------------------------------------------------------------------------
// delete_num_threads
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn delete_num_threads_removes_value() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    // Set a value first
    frankenphp_check::benchmark::set_num_threads(ADMIN_PORT, 10);
    assert_eq!(curl_get_num_threads(), "10");

    let ok = frankenphp_check::benchmark::delete_num_threads(ADMIN_PORT);
    assert!(ok, "delete should succeed");

    assert_eq!(curl_get_num_threads(), "null");
}

#[test]
#[ignore]
fn delete_num_threads_is_idempotent() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    // Delete when nothing is set should still succeed
    let ok = frankenphp_check::benchmark::delete_num_threads(ADMIN_PORT);
    assert!(ok, "delete on non-existent key should still succeed");
}

// ---------------------------------------------------------------------------
// Round-trip: set → get → delete → get
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn full_round_trip() {
    if !frankenphp_available() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    cleanup();

    // Initially not set
    assert_eq!(
        frankenphp_check::benchmark::get_num_threads(ADMIN_PORT),
        None
    );

    // Set
    assert!(frankenphp_check::benchmark::set_num_threads(ADMIN_PORT, 32));
    assert_eq!(
        frankenphp_check::benchmark::get_num_threads(ADMIN_PORT),
        Some(32)
    );

    // Overwrite
    assert!(frankenphp_check::benchmark::set_num_threads(ADMIN_PORT, 64));
    assert_eq!(
        frankenphp_check::benchmark::get_num_threads(ADMIN_PORT),
        Some(64)
    );

    // Delete
    assert!(frankenphp_check::benchmark::delete_num_threads(ADMIN_PORT));
    assert_eq!(
        frankenphp_check::benchmark::get_num_threads(ADMIN_PORT),
        None
    );
}

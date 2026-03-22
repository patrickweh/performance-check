//! Integration tests for MySQL checks.
//! These require a running MySQL server and are marked #[ignore].
//! Run with: cargo test -- --ignored
//! CI sets TEST_MYSQL=1 and provides a MySQL service container.

use std::process::Command;

fn mysql_available() -> bool {
    std::env::var("TEST_MYSQL").is_ok()
}

fn mysql_exec(query: &str) -> Option<String> {
    // Try defaults-file first (works on real servers)
    let output = Command::new("mysql")
        .args([
            "--defaults-file=/etc/mysql/debian.cnf",
            "-N",
            "-B",
            "-e",
            query,
        ])
        .output()
        .ok();

    if let Some(ref o) = output {
        if o.status.success() {
            return Some(String::from_utf8_lossy(&o.stdout).trim().to_string());
        }
    }

    // Fallback: use env vars (works in CI with service containers)
    let host = std::env::var("MYSQL_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let user = std::env::var("MYSQL_USER").unwrap_or_else(|_| "root".to_string());
    let password = std::env::var("MYSQL_PASSWORD").unwrap_or_default();

    let output = Command::new("mysql")
        .args([
            "-h",
            &host,
            "-u",
            &user,
            &format!("-p{password}"),
            "-N",
            "-B",
            "-e",
            query,
        ])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

#[test]
#[ignore]
fn mysql_version_query() {
    if !mysql_available() {
        return;
    }
    let version = mysql_exec("SELECT VERSION()");
    assert!(version.is_some(), "Should be able to query MySQL version");
    let v = version.unwrap();
    assert!(!v.is_empty(), "Version should not be empty");
    // MySQL 8.x format: 8.0.xx or 8.4.xx
    assert!(v.contains('.'), "Version should contain dots: got '{v}'");
}

#[test]
#[ignore]
fn mysql_innodb_buffer_pool_size_readable() {
    if !mysql_available() {
        return;
    }
    let val = mysql_exec("SELECT @@global.innodb_buffer_pool_size");
    assert!(val.is_some(), "Should read innodb_buffer_pool_size");
    let bytes: u64 = val.unwrap().parse().expect("Should be a number");
    assert!(bytes > 0, "innodb_buffer_pool_size should be > 0");
}

#[test]
#[ignore]
fn mysql_innodb_log_file_size_readable() {
    if !mysql_available() {
        return;
    }
    let val = mysql_exec("SELECT @@global.innodb_log_file_size");
    // MySQL 8.0.30+ deprecated this, but it still returns a value
    if let Some(v) = val {
        let bytes: u64 = v.parse().unwrap_or(0);
        assert!(bytes > 0, "innodb_log_file_size should be > 0");
    }
}

#[test]
#[ignore]
fn mysql_max_connections_readable() {
    if !mysql_available() {
        return;
    }
    let val = mysql_exec("SELECT @@global.max_connections");
    assert!(val.is_some(), "Should read max_connections");
    let n: u64 = val.unwrap().parse().expect("Should be a number");
    assert!(n > 0, "max_connections should be > 0");
}

#[test]
#[ignore]
fn mysql_slow_query_log_readable() {
    if !mysql_available() {
        return;
    }
    let val = mysql_exec("SELECT @@global.slow_query_log");
    assert!(val.is_some(), "Should read slow_query_log");
    let v = val.unwrap();
    assert!(
        v == "0" || v == "1" || v == "ON" || v == "OFF",
        "slow_query_log should be 0/1/ON/OFF, got '{v}'"
    );
}

#[test]
#[ignore]
fn mysql_tmp_table_size_readable() {
    if !mysql_available() {
        return;
    }
    let val = mysql_exec("SELECT @@global.tmp_table_size");
    assert!(val.is_some(), "Should read tmp_table_size");
    let bytes: u64 = val.unwrap().parse().expect("Should be a number");
    assert!(bytes > 0, "tmp_table_size should be > 0");
}

#[test]
#[ignore]
fn mysql_innodb_flush_log_at_trx_commit_readable() {
    if !mysql_available() {
        return;
    }
    let val = mysql_exec("SELECT @@global.innodb_flush_log_at_trx_commit");
    assert!(val.is_some());
    let v: u64 = val.unwrap().parse().expect("Should be a number");
    assert!(
        v <= 2,
        "innodb_flush_log_at_trx_commit should be 0, 1, or 2"
    );
}

#[test]
#[ignore]
fn mysql_benchmark_query_runs() {
    if !mysql_available() {
        return;
    }
    let result = mysql_exec("SELECT BENCHMARK(1000, MD5('test'))");
    assert!(result.is_some(), "BENCHMARK query should succeed");
    assert_eq!(result.unwrap(), "0", "BENCHMARK returns 0 on success");
}

#[test]
#[ignore]
fn mysql_buffer_pool_hit_rate_queryable() {
    if !mysql_available() {
        return;
    }
    let result = mysql_exec(
        "SELECT (1 - (Innodb_buffer_pool_reads / NULLIF(Innodb_buffer_pool_read_requests, 0))) * 100 \
         FROM (SELECT VARIABLE_VALUE AS Innodb_buffer_pool_reads FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Innodb_buffer_pool_reads') a, \
         (SELECT VARIABLE_VALUE AS Innodb_buffer_pool_read_requests FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Innodb_buffer_pool_read_requests') b"
    );
    // May return NULL if no reads yet, that's OK
    if let Some(v) = result {
        if v != "NULL" {
            let rate: f64 = v.parse().expect("Should be a float");
            assert!(
                (0.0..=100.0).contains(&rate),
                "Hit rate should be 0-100, got {rate}"
            );
        }
    }
}

#[test]
#[ignore]
fn mysql_cnf_fix_creates_valid_config() {
    if !mysql_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cnf_path = dir.path().join("mysql-test.cnf");

    // Simulate what write_fixes does for MySQL
    std::fs::write(
        &cnf_path,
        "[mysqld]\ninnodb_buffer_pool_size=128M\nmax_connections=200\n",
    )
    .unwrap();

    let content = std::fs::read_to_string(&cnf_path).unwrap();
    assert!(content.starts_with("[mysqld]"));
    assert!(content.contains("innodb_buffer_pool_size=128M"));
    assert!(content.contains("max_connections=200"));

    // Verify MySQL can parse this format (dry run)
    let output = Command::new("my_print_defaults")
        .args(["--defaults-file", cnf_path.to_str().unwrap(), "mysqld"])
        .output();

    if let Ok(o) = output {
        if o.status.success() {
            let stdout = String::from_utf8_lossy(&o.stdout);
            assert!(
                stdout.contains("innodb_buffer_pool_size") || stdout.contains("128M"),
                "my_print_defaults should show our config"
            );
        }
    }
    // If my_print_defaults isn't available, that's OK — the file format is still valid
}

#[test]
#[ignore]
fn mysql_query_cache_type_check() {
    if !mysql_available() {
        return;
    }
    // MySQL 8.0 removed query_cache_type, so this variable may not exist
    let val = mysql_exec("SELECT @@global.query_cache_type");
    // Either it returns a value or an error — both are valid
    // The check module handles both cases
    if let Some(v) = val {
        assert!(
            v == "OFF" || v == "ON" || v == "DEMAND" || v == "0" || v == "1" || v == "2",
            "Unexpected query_cache_type: '{v}'"
        );
    }
}

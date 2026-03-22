use crate::types::{CheckResult, SystemContext};
use std::fs;
use std::process::Command;

pub fn check(ctx: &SystemContext) -> Vec<CheckResult> {
    if !ctx.mysql_running {
        return vec![CheckResult::info(
            "MySQL/MariaDB",
            "Not running locally — skipping",
        )];
    }

    let mut results = Vec::new();

    // Detect MySQL config file
    let cnf_file = detect_mysql_cnf();
    if let Some(ref f) = cnf_file {
        results.push(CheckResult::info("MySQL Config", f.clone()));
    } else {
        results.push(CheckResult::warn(
            "MySQL Config",
            "No .cnf with [mysqld] found — fixes will create /etc/mysql/conf.d/custom.cnf",
        ));
    }

    // Use detected file or fall back to a sensible default
    let cnf = cnf_file.unwrap_or_else(|| "/etc/mysql/conf.d/custom.cnf".to_string());

    // Detect MySQL version
    let version = mysql_query("SELECT VERSION()");
    let mysql_major = version
        .as_ref()
        .and_then(|v| v.split('.').next())
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let is_mariadb = version
        .as_ref()
        .is_some_and(|v| v.to_lowercase().contains("mariadb"));

    if let Some(ref v) = version {
        results.push(CheckResult::info("MySQL Version", v.clone()));
    }

    // Calculate recommended innodb_buffer_pool_size
    let mysql_ram_budget = ctx.mysql_ram_mb;
    let recommended_pool = mysql_ram_budget * 75 / 100;
    let recommended_pool_mb = recommended_pool.max(128);

    check_mysql_var(
        "innodb_buffer_pool_size",
        recommended_pool_mb * 1024 * 1024,
        &format!("≥{recommended_pool_mb}MB (75% of MySQL RAM share)"),
        &cnf,
        &mut results,
    );

    check_mysql_bytes(
        "innodb_log_file_size",
        256 * 1024 * 1024,
        "≥256MB",
        &cnf,
        &mut results,
    );

    // query_cache_type — only relevant for MySQL < 8.0 and MariaDB
    if is_mariadb || mysql_major < 8 {
        let val = mysql_variable("query_cache_type");
        match val.as_deref() {
            Some("OFF") | Some("0") => {
                results.push(CheckResult::ok("query_cache_type", "OFF"));
            }
            Some(v) => {
                results.push(
                    CheckResult::warn(
                        "query_cache_type",
                        format!("'{v}' — should be OFF (global mutex on every write)"),
                    )
                    .with_fix(
                        "Set query_cache_type=0",
                        &cnf,
                        "query_cache_type=0",
                    ),
                );
            }
            None => {}
        }
    }

    // slow_query_log
    let slow_log = mysql_variable("slow_query_log");
    match slow_log.as_deref() {
        Some("ON") | Some("1") => {
            results.push(CheckResult::ok("slow_query_log", "ON"));
            let long_qt = mysql_variable("long_query_time");
            if let Some(ref v) = long_qt {
                let secs: f64 = v.parse().unwrap_or(10.0);
                if secs > 1.0 {
                    results.push(
                        CheckResult::warn("long_query_time", format!("{v}s — recommend ≤1s"))
                            .with_fix("Set long_query_time=1", &cnf, "long_query_time=1"),
                    );
                } else {
                    results.push(CheckResult::ok("long_query_time", format!("{v}s")));
                }
            }
        }
        _ => {
            results.push(
                CheckResult::warn("slow_query_log", "OFF — recommend enabling").with_fix(
                    "Enable slow query log",
                    &cnf,
                    "slow_query_log=1\nlong_query_time=1",
                ),
            );
        }
    }

    // max_connections
    let max_conn = mysql_variable("max_connections")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if max_conn >= 100 {
        results.push(CheckResult::ok("max_connections", format!("{max_conn}")));
    } else {
        results.push(
            CheckResult::warn("max_connections", format!("{max_conn} — recommend ≥100")).with_fix(
                "Set max_connections=200",
                &cnf,
                "max_connections=200",
            ),
        );
    }

    // innodb_flush_log_at_trx_commit
    let flush = mysql_variable("innodb_flush_log_at_trx_commit");
    match flush.as_deref() {
        Some("1") => {
            results.push(CheckResult::ok(
                "innodb_flush_log_at_trx_commit",
                "1 (full durability)",
            ));
        }
        Some(v) => {
            results.push(CheckResult::warn(
                "innodb_flush_log_at_trx_commit",
                format!("'{v}' — value 2 risks data loss on crash"),
            ));
        }
        None => {}
    }

    // tmp_table_size
    check_mysql_bytes(
        "tmp_table_size",
        64 * 1024 * 1024,
        "≥64MB",
        &cnf,
        &mut results,
    );

    results
}

/// Auto-detect MySQL config file in /etc/mysql/conf.d/.
fn detect_mysql_cnf() -> Option<String> {
    // Prefer /etc/mysql/my.cnf if it contains a [mysqld] section
    // (this is the main config file on most Debian/Ubuntu setups)
    let my_cnf = "/etc/mysql/my.cnf";
    if let Ok(content) = fs::read_to_string(my_cnf) {
        if content.contains("[mysqld]") {
            return Some(my_cnf.to_string());
        }
    }

    // Fall back to files in /etc/mysql/conf.d/
    let conf_dir = "/etc/mysql/conf.d";
    let entries = match fs::read_dir(conf_dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    let mut cnf_files: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".cnf") {
                Some(format!("{conf_dir}/{name}"))
            } else {
                None
            }
        })
        .collect();

    cnf_files.sort();
    cnf_files.into_iter().next()
}

fn mysql_query(query: &str) -> Option<String> {
    let output = Command::new("mysql")
        .args([
            "--defaults-file=/etc/mysql/debian.cnf",
            "-N",
            "-B",
            "-e",
            query,
        ])
        .output()
        .or_else(|_| {
            Command::new("mysql")
                .args(["-u", "root", "-N", "-B", "-e", query])
                .output()
        })
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn mysql_variable(name: &str) -> Option<String> {
    mysql_query(&format!("SELECT @@global.{name}"))
}

fn check_mysql_var(
    name: &str,
    min_bytes: u64,
    hint: &str,
    cnf: &str,
    results: &mut Vec<CheckResult>,
) {
    let val = mysql_variable(name);
    match val {
        Some(v) => {
            let numeric: u64 = v.parse().unwrap_or(0);
            if numeric >= min_bytes {
                results.push(CheckResult::ok(name, format_bytes(numeric)));
            } else {
                let recommended = format_bytes(min_bytes);
                results.push(
                    CheckResult::warn(
                        name,
                        format!("{} — recommend {hint}", format_bytes(numeric)),
                    )
                    .with_fix(
                        format!("Set {name}={recommended}"),
                        cnf,
                        format!("{name}={recommended}"),
                    ),
                );
            }
        }
        None => {
            results.push(CheckResult::warn(name, "Could not read"));
        }
    }
}

fn check_mysql_bytes(
    name: &str,
    min_bytes: u64,
    hint: &str,
    cnf: &str,
    results: &mut Vec<CheckResult>,
) {
    check_mysql_var(name, min_bytes, hint, cnf, results);
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{}G", bytes / (1024 * 1024 * 1024))
    } else if bytes >= 1024 * 1024 {
        format!("{}M", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{}K", bytes / 1024)
    } else {
        format!("{bytes}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_gigabytes() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1G");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2G");
    }

    #[test]
    fn format_bytes_megabytes() {
        assert_eq!(format_bytes(256 * 1024 * 1024), "256M");
        assert_eq!(format_bytes(768 * 1024 * 1024), "768M");
    }

    #[test]
    fn format_bytes_kilobytes() {
        assert_eq!(format_bytes(512 * 1024), "512K");
        // 4096K = 4M, so format_bytes prefers the larger unit
        assert_eq!(format_bytes(4096 * 1024), "4M");
    }

    #[test]
    fn format_bytes_plain() {
        assert_eq!(format_bytes(512), "512");
        assert_eq!(format_bytes(0), "0");
    }

    #[test]
    fn detect_mysql_cnf_nonexistent_dir() {
        let result = detect_mysql_cnf();
        let _ = result;
    }

    // --- Additional format_bytes edge cases ---

    #[test]
    fn format_bytes_boundary_just_below_megabyte() {
        // 1023K should stay as K
        assert_eq!(format_bytes(1023 * 1024), "1023K");
    }

    #[test]
    fn format_bytes_boundary_exactly_megabyte() {
        assert_eq!(format_bytes(1024 * 1024), "1M");
    }

    #[test]
    fn format_bytes_boundary_just_below_gigabyte() {
        assert_eq!(format_bytes(1023 * 1024 * 1024), "1023M");
    }

    #[test]
    fn format_bytes_boundary_exactly_gigabyte() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1G");
    }

    #[test]
    fn format_bytes_just_below_kilobyte() {
        assert_eq!(format_bytes(1023), "1023");
    }

    #[test]
    fn format_bytes_exactly_kilobyte() {
        assert_eq!(format_bytes(1024), "1K");
    }

    #[test]
    fn format_bytes_common_mysql_values() {
        // innodb_buffer_pool_size typical values
        assert_eq!(format_bytes(128 * 1024 * 1024), "128M");
        assert_eq!(format_bytes(256 * 1024 * 1024), "256M");
        assert_eq!(format_bytes(512 * 1024 * 1024), "512M");
        assert_eq!(format_bytes(768 * 1024 * 1024), "768M");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1G");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2G");
        assert_eq!(format_bytes(4 * 1024 * 1024 * 1024), "4G");
    }

    #[test]
    fn format_bytes_innodb_log_file_size() {
        assert_eq!(format_bytes(48 * 1024 * 1024), "48M");
        assert_eq!(format_bytes(256 * 1024 * 1024), "256M");
        assert_eq!(format_bytes(512 * 1024 * 1024), "512M");
    }

    #[test]
    fn format_bytes_tmp_table_size() {
        assert_eq!(format_bytes(16 * 1024 * 1024), "16M");
        assert_eq!(format_bytes(64 * 1024 * 1024), "64M");
    }

    #[test]
    fn format_bytes_one_byte() {
        assert_eq!(format_bytes(1), "1");
    }

    #[test]
    fn format_bytes_integer_truncation() {
        // 1.5G in bytes = 1610612736 → should show 1G (integer division)
        assert_eq!(format_bytes(1024 * 1024 * 1024 + 512 * 1024 * 1024), "1G");
    }
}

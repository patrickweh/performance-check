use crate::types::{CheckResult, SystemContext};
use std::fs;
use std::path::Path;

pub fn gather(app_path: &str) -> (SystemContext, Vec<CheckResult>) {
    let mut results = Vec::new();

    let cpu_cores = num_cpus();
    let (total_ram_mb, available_ram_mb, swap_used_mb) = read_meminfo();

    results.push(CheckResult::info("CPU Cores", format!("{cpu_cores}")));
    results.push(CheckResult::info(
        "Memory",
        format!("Total: {total_ram_mb}MB, Available: {available_ram_mb}MB"),
    ));

    if swap_used_mb > 0 {
        results.push(CheckResult::warn(
            "Swap Usage",
            format!("{swap_used_mb}MB swap in use — indicates memory pressure"),
        ));
    } else {
        results.push(CheckResult::ok("Swap Usage", "No swap in use"));
    }

    let (mysql_running, mysql_pid, mysql_ram_mb) = detect_service(&["mysqld", "mariadbd"]);
    let (redis_running, redis_pid, redis_ram_mb) = detect_service(&["redis-server"]);

    if mysql_running {
        results.push(CheckResult::info(
            "MySQL/MariaDB",
            format!("Running (PID {}, ~{mysql_ram_mb}MB RSS)", mysql_pid.unwrap_or(0)),
        ));
    }
    if redis_running {
        results.push(CheckResult::info(
            "Redis",
            format!("Running (PID {}, ~{redis_ram_mb}MB RSS)", redis_pid.unwrap_or(0)),
        ));
    }

    let os_reserve: u64 = 512;
    let buffer = total_ram_mb / 10;
    let php_ram_budget_mb = total_ram_mb
        .saturating_sub(os_reserve)
        .saturating_sub(mysql_ram_mb)
        .saturating_sub(redis_ram_mb)
        .saturating_sub(buffer);

    results.push(CheckResult::info(
        "PHP RAM Budget",
        format!(
            "{php_ram_budget_mb}MB (Total {total_ram_mb} - OS 512 - MySQL {mysql_ram_mb} - Redis {redis_ram_mb} - 10% buffer {buffer})"
        ),
    ));

    let (laravel_version, laravel_major) = detect_laravel_version(app_path);
    if let Some(ref v) = laravel_version {
        results.push(CheckResult::info("Laravel Version", v.clone()));
    } else {
        results.push(CheckResult::warn(
            "Laravel Version",
            "Could not detect Laravel version",
        ));
    }

    let ctx = SystemContext {
        cpu_cores,
        total_ram_mb,
        available_ram_mb,
        swap_used_mb,
        mysql_running,
        mysql_pid,
        mysql_ram_mb,
        redis_running,
        redis_pid,
        redis_ram_mb,
        php_ram_budget_mb,
        laravel_version,
        laravel_major,
    };

    (ctx, results)
}

fn num_cpus() -> usize {
    fs::read_to_string("/proc/cpuinfo")
        .map(|s| s.matches("processor\t").count())
        .unwrap_or(1)
}

fn read_meminfo() -> (u64, u64, u64) {
    let content = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let get = |key: &str| -> u64 {
        content
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            / 1024 // kB -> MB
    };
    let total = get("MemTotal:");
    let available = get("MemAvailable:");
    let swap_total = get("SwapTotal:");
    let swap_free = get("SwapFree:");
    let swap_used = swap_total.saturating_sub(swap_free);
    (total, available, swap_used)
}

fn detect_service(names: &[&str]) -> (bool, Option<u32>, u64) {
    let Ok(entries) = fs::read_dir("/proc") else {
        return (false, None, 0);
    };

    for entry in entries.flatten() {
        let pid_str = entry.file_name();
        let pid_str = pid_str.to_string_lossy();
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };

        let comm_path = format!("/proc/{pid}/comm");
        let Ok(comm) = fs::read_to_string(&comm_path) else {
            continue;
        };
        let comm = comm.trim();

        if names.iter().any(|n| comm == *n) {
            let rss_mb = read_process_rss(pid);
            return (true, Some(pid), rss_mb);
        }
    }

    (false, None, 0)
}

fn read_process_rss(pid: u32) -> u64 {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        / 1024 // kB -> MB
}

fn detect_laravel_version(app_path: &str) -> (Option<String>, Option<u32>) {
    let app_file = Path::new(app_path)
        .join("vendor/laravel/framework/src/Illuminate/Foundation/Application.php");
    let content = match fs::read_to_string(app_file) {
        Ok(c) => c,
        Err(_) => return (None, None),
    };

    let re = match regex::Regex::new(r"const\s+VERSION\s*=\s*'([^']+)'") {
        Ok(r) => r,
        Err(_) => return (None, None),
    };
    let caps = match re.captures(&content) {
        Some(c) => c,
        None => return (None, None),
    };
    let version = match caps.get(1) {
        Some(m) => m.as_str().to_string(),
        None => return (None, None),
    };
    let major = version.split('.').next().and_then(|v| v.parse::<u32>().ok());
    (Some(version), major)
}

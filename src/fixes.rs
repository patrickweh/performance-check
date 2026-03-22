use crate::benchmark::{self, BenchmarkKind};
use crate::types::{CheckResult, Status, SystemContext};
use colored::Colorize;
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

/// Interactively propose fixes for WARN/FAIL results that have fixable actions.
/// For auto-applicable fixes: runs a fix-specific benchmark before and after,
/// then asks the user whether to keep the fix or restore the original file.
pub fn propose_interactive_fixes(results: &[CheckResult], frankenphp_bin: &str, app_path: &str) {
    let fixable: Vec<_> = results
        .iter()
        .filter(|r| r.fix.is_some() && matches!(r.status, Status::Warn | Status::Fail))
        .collect();

    if fixable.is_empty() {
        return;
    }

    // Group fixes by target file
    let mut by_file: BTreeMap<String, Vec<(&CheckResult, &str)>> = BTreeMap::new();
    for r in &fixable {
        let fix = r.fix.as_ref().unwrap();
        by_file
            .entry(fix.file.clone())
            .or_default()
            .push((r, &fix.content));
    }

    println!();
    println!("  {}", "Fix Suggestions".bold().underline());
    println!();

    // Separate fixes into auto-applicable categories
    let mut auto_fixable: BTreeMap<String, Vec<(&CheckResult, &str)>> = BTreeMap::new();
    let mut redis_fixes: Vec<(&CheckResult, &str)> = Vec::new();
    let mut systemd_fixes: BTreeMap<String, Vec<(&CheckResult, &str)>> = BTreeMap::new();
    let mut manual_fixes: Vec<(&CheckResult, &str)> = Vec::new();

    for (file, entries) in &by_file {
        if file == "redis-cli" {
            redis_fixes.extend(entries.iter().copied());
        } else if file.contains("systemd") && file.ends_with("override.conf") {
            systemd_fixes.insert(file.clone(), entries.clone());
        } else if file.starts_with('/') && !file.contains('\u{2192}') {
            auto_fixable.insert(file.clone(), entries.clone());
        } else {
            for entry in entries {
                manual_fixes.push(*entry);
            }
        }
    }

    // Handle auto-fixable files with benchmark before/after
    for (file, entries) in &auto_fixable {
        let kind = BenchmarkKind::from_file(file);

        println!("    {}", file.bold());
        println!();

        for (i, (r, content)) in entries.iter().enumerate() {
            let status_badge = match r.status {
                Status::Warn => " WARN ".on_yellow().black().bold(),
                Status::Fail => " FAIL ".on_red().white().bold(),
                _ => " ---- ".dimmed(),
            };

            println!(
                "      {}. {} {}",
                format!("{}", i + 1).dimmed(),
                status_badge,
                r.label.bold()
            );
            println!("         {} {}", "\u{2192}".dimmed(), content.cyan());
        }
        println!();

        let bench_available = !matches!(kind, BenchmarkKind::None);

        let items: &[&str] = if bench_available {
            &["Try with benchmark", "Apply without benchmark", "Skip"]
        } else {
            &["Apply", "Skip"]
        };

        let selection = dialoguer::Select::new()
            .with_prompt(format!(
                "    {}",
                if bench_available {
                    format!(
                        "Apply {} fix(es)? (benchmark: {})",
                        entries.len(),
                        kind.label()
                    )
                } else {
                    format!("Apply {} fix(es)?", entries.len())
                }
            ))
            .items(items)
            .default(0)
            .interact();

        if bench_available {
            match selection {
                Ok(0) => {
                    apply_with_benchmark(file, entries, frankenphp_bin, app_path, kind);
                }
                Ok(1) => {
                    apply_file_fixes(file, entries);
                }
                _ => {
                    println!("    {}", "Skipped.".dimmed());
                }
            }
        } else {
            match selection {
                Ok(0) => {
                    apply_file_fixes(file, entries);
                }
                _ => {
                    println!("    {}", "Skipped.".dimmed());
                }
            }
        }
        println!();
    }

    // Handle Redis fixes
    if !redis_fixes.is_empty() {
        println!("    {}", "Redis Configuration".bold());
        println!();

        for (i, (r, content)) in redis_fixes.iter().enumerate() {
            let status_badge = match r.status {
                Status::Warn => " WARN ".on_yellow().black().bold(),
                Status::Fail => " FAIL ".on_red().white().bold(),
                _ => " ---- ".dimmed(),
            };

            println!(
                "      {}. {} {}",
                format!("{}", i + 1).dimmed(),
                status_badge,
                r.label.bold()
            );
            println!(
                "         {} redis-cli CONFIG SET {}",
                "\u{2192}".dimmed(),
                content.cyan()
            );
        }
        println!();

        let selection = dialoguer::Select::new()
            .with_prompt(format!(
                "    Apply {} Redis fix(es) via redis-cli?",
                redis_fixes.len()
            ))
            .items(&["Apply", "Skip"])
            .default(0)
            .interact();

        match selection {
            Ok(0) => {
                apply_redis_fixes(&redis_fixes);
            }
            _ => {
                println!("    {}", "Skipped.".dimmed());
            }
        }
        println!();
    }

    // Handle systemd environment fixes (GODEBUG, GOMEMLIMIT)
    for (file, entries) in &systemd_fixes {
        println!("    {}", "FrankenPHP Service Environment".bold());
        println!();

        for (i, (r, content)) in entries.iter().enumerate() {
            let status_badge = match r.status {
                Status::Warn => " WARN ".on_yellow().black().bold(),
                Status::Fail => " FAIL ".on_red().white().bold(),
                _ => " ---- ".dimmed(),
            };

            println!(
                "      {}. {} {}",
                format!("{}", i + 1).dimmed(),
                status_badge,
                r.label.bold()
            );
            println!("         {} {}", "\u{2192}".dimmed(), content.cyan());
        }
        println!();

        let selection = dialoguer::Select::new()
            .with_prompt(format!(
                "    Apply {} fix(es) to systemd service override?",
                entries.len()
            ))
            .items(&["Apply", "Skip"])
            .default(0)
            .interact();

        match selection {
            Ok(0) => {
                apply_systemd_env_fixes(file, entries);
            }
            _ => {
                println!("    {}", "Skipped.".dimmed());
            }
        }
        println!();
    }

    // Show manual fixes
    if !manual_fixes.is_empty() {
        println!("    {}", "Manual Actions Required".bold());
        println!();
        for (r, content) in &manual_fixes {
            let fix = r.fix.as_ref().unwrap();
            let status_badge = match r.status {
                Status::Warn => " WARN ".on_yellow().black().bold(),
                Status::Fail => " FAIL ".on_red().white().bold(),
                _ => " ---- ".dimmed(),
            };

            println!(
                "      {} {} {}",
                status_badge,
                r.label.bold(),
                format!("({})", fix.description).dimmed()
            );
            println!("         {}", content.cyan());
            println!();
        }
    }
}

fn apply_with_benchmark(
    file: &str,
    entries: &[(&CheckResult, &str)],
    frankenphp_bin: &str,
    app_path: &str,
    kind: BenchmarkKind,
) {
    // 1. Create backup (full file content, or None if file doesn't exist yet)
    let backup = fs::read_to_string(file).ok();

    // 2. Benchmark BEFORE
    println!();
    println!(
        "    {} Running {} benchmark (before)...",
        "\u{25B6}".cyan(),
        kind.label()
    );

    let before = benchmark::run(kind, frankenphp_bin, app_path, 3);

    if let Some(ref b) = before {
        b.display_summary();
    } else {
        println!(
            "    {}",
            "Benchmark unavailable - applying fix without comparison.".yellow()
        );
        apply_file_fixes(file, entries);
        return;
    }

    // 3. Apply fix temporarily
    println!();
    println!("    {} Applying fix temporarily...", "\u{25B6}".cyan());
    apply_file_fixes_silent(file, entries);

    // 4. Benchmark AFTER
    println!(
        "    {} Running {} benchmark (after)...",
        "\u{25B6}".cyan(),
        kind.label()
    );

    let after = benchmark::run(kind, frankenphp_bin, app_path, 3);

    if let Some(ref a) = after {
        a.display_summary();
    }

    // 5. Show comparison
    if let (Some(ref b), Some(ref a)) = (&before, &after) {
        benchmark::BenchmarkResult::display_comparison(b, a);
    }

    // 6. Ask user: keep or revert?
    let keep = dialoguer::Select::new()
        .with_prompt("    Keep the applied fix?")
        .items(&["Keep fix", "Revert to original"])
        .default(0)
        .interact();

    match keep {
        Ok(0) => {
            println!("    {}", "Fix applied permanently.".green().bold());
            print_restart_hint(file);
        }
        _ => {
            // Restore from backup
            match &backup {
                Some(content) => match fs::write(file, content) {
                    Ok(_) => {
                        println!("    {}", format!("Restored original {file}").green());
                    }
                    Err(e) => {
                        println!("    {}", format!("Failed to restore {file}: {e}").red());
                        println!("    {}", "Backup content:".red());
                        for line in content.lines().take(20) {
                            println!("      {}", line.red());
                        }
                    }
                },
                None => match fs::remove_file(file) {
                    Ok(_) => {
                        println!(
                            "    {}",
                            format!("Removed {file} (did not exist before)").green()
                        );
                    }
                    Err(e) => {
                        println!("    {}", format!("Failed to remove {file}: {e}").red());
                    }
                },
            }
        }
    }
}

/// Apply fixes and print status messages.
fn apply_file_fixes(file: &str, entries: &[(&CheckResult, &str)]) {
    let applied = write_fixes(file, entries);
    match applied {
        Ok(count) => {
            println!(
                "    {}",
                format!("Applied {count} fix(es) to {file}").green().bold()
            );
            print_restart_hint(file);
        }
        Err(e) => {
            println!("    {}", format!("Failed to write {file}: {e}").red());
            println!("    {}", "Try running with sudo".yellow());
        }
    }
}

/// Apply fixes silently (no output). Used during benchmark flow.
fn apply_file_fixes_silent(file: &str, entries: &[(&CheckResult, &str)]) {
    let _ = write_fixes(file, entries);
}

/// Write fixes to a file. Returns the number of applied fixes.
fn write_fixes(file: &str, entries: &[(&CheckResult, &str)]) -> Result<usize, std::io::Error> {
    let mut content = fs::read_to_string(file).unwrap_or_default();
    let mut applied = 0;

    // MySQL .cnf files need a [mysqld] section header for settings to be recognized
    let is_mysql_cnf =
        file.ends_with(".cnf") && (file.contains("mysql") || file.contains("mariadb"));
    if is_mysql_cnf && !content.contains("[mysqld]") {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("[mysqld]\n");
    }

    for (_r, fix_line) in entries {
        for line in fix_line.lines() {
            if let Some((key, _value)) = line.split_once('=') {
                let key = key.trim();
                let mut found = false;
                let new_content: Vec<String> = content
                    .lines()
                    .map(|l| {
                        let trimmed = l.trim();
                        if (trimmed.starts_with(key)
                            && trimmed[key.len()..].trim_start().starts_with('='))
                            || trimmed.starts_with(&format!(";{key}"))
                            || trimmed.starts_with(&format!("; {key}"))
                        {
                            found = true;
                            line.to_string()
                        } else {
                            l.to_string()
                        }
                    })
                    .collect();

                if found {
                    content = new_content.join("\n");
                } else {
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(line);
                    content.push('\n');
                }
                applied += 1;
            }
        }
    }

    fs::write(file, &content)?;
    Ok(applied)
}

/// Variables that can NOT be changed at runtime via SET GLOBAL (require restart).
const STATIC_MYSQL_VARS: &[&str] = &["innodb_log_file_size"];

/// Parse a CNF value suffix (M, G, K) into bytes for SET GLOBAL.
fn cnf_value_to_set_global(value: &str) -> String {
    let value = value.trim();
    if let Some(num) = value.strip_suffix('G') {
        if let Ok(n) = num.parse::<u64>() {
            return (n * 1024 * 1024 * 1024).to_string();
        }
    }
    if let Some(num) = value.strip_suffix('M') {
        if let Ok(n) = num.parse::<u64>() {
            return (n * 1024 * 1024).to_string();
        }
    }
    if let Some(num) = value.strip_suffix('K') {
        if let Ok(n) = num.parse::<u64>() {
            return (n * 1024).to_string();
        }
    }
    value.to_string()
}

/// Apply MySQL variables at runtime via SET GLOBAL.
/// Returns a list of (variable, old_value) for rollback.
fn apply_mysql_runtime(entries: &[(&CheckResult, &str)]) -> Vec<(String, String)> {
    let mut backups = Vec::new();

    for (_r, fix_content) in entries {
        for line in fix_content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                if STATIC_MYSQL_VARS.contains(&key) {
                    continue;
                }

                // Backup current value
                let backup_query = format!("SELECT @@global.{key}");
                let old_value = mysql_query_internal(&backup_query);
                if let Some(ref old) = old_value {
                    backups.push((key.to_string(), old.clone()));
                }

                // Apply via SET GLOBAL
                let set_value = cnf_value_to_set_global(value);
                let set_query = format!("SET GLOBAL {key} = {set_value}");
                let _ = mysql_query_internal(&set_query);
            }
        }
    }

    backups
}

/// Apply MySQL variables at runtime silently (no output). Used during benchmark flow.
fn apply_mysql_runtime_silent(entries: &[(&CheckResult, &str)]) -> Vec<(String, String)> {
    apply_mysql_runtime(entries)
}

/// Restore MySQL runtime variables from backups.
fn restore_mysql_runtime(backups: &[(String, String)]) {
    for (key, value) in backups {
        let set_value = cnf_value_to_set_global(value);
        let set_query = format!("SET GLOBAL {key} = {set_value}");
        let _ = mysql_query_internal(&set_query);
    }
}

/// Internal mysql query helper (reuses the same connection logic as checks).
fn mysql_query_internal(query: &str) -> Option<String> {
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

/// Apply Redis fixes via redis-cli CONFIG SET, then CONFIG REWRITE to persist.
fn apply_redis_fixes(entries: &[(&CheckResult, &str)]) {
    let mut any_failed = false;

    for (_r, content) in entries {
        // content is "maxmemory 2397mb" or "maxmemory-policy allkeys-lru"
        let parts: Vec<&str> = content.splitn(2, ' ').collect();
        if parts.len() != 2 {
            println!(
                "    {}",
                format!("Invalid redis fix format: {content}").red()
            );
            any_failed = true;
            continue;
        }

        let output = Command::new("redis-cli")
            .args(["CONFIG", "SET", parts[0], parts[1]])
            .output();

        match output {
            Ok(o) if o.status.success() => {
                let response = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if response == "OK" {
                    println!("    {}", format!("Set {} = {}", parts[0], parts[1]).green());
                } else {
                    println!(
                        "    {}",
                        format!(
                            "redis-cli CONFIG SET {} {}: {}",
                            parts[0], parts[1], response
                        )
                        .red()
                    );
                    any_failed = true;
                }
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                println!(
                    "    {}",
                    format!(
                        "Failed: redis-cli CONFIG SET {} {}: {}",
                        parts[0], parts[1], err
                    )
                    .red()
                );
                any_failed = true;
            }
            Err(e) => {
                println!("    {}", format!("Could not run redis-cli: {e}").red());
                any_failed = true;
            }
        }
    }

    if !any_failed {
        // Persist changes to redis.conf
        let rewrite = Command::new("redis-cli")
            .args(["CONFIG", "REWRITE"])
            .output();

        match rewrite {
            Ok(o) if o.status.success() => {
                println!(
                    "    {}",
                    "Redis configuration persisted (CONFIG REWRITE)."
                        .green()
                        .bold()
                );
            }
            _ => {
                println!(
                    "    {}",
                    "Warning: Could not persist via CONFIG REWRITE. Changes are active but may not survive a Redis restart.".yellow()
                );
            }
        }
    }
}

/// Apply environment variable fixes to a systemd service override file.
fn apply_systemd_env_fixes(override_path: &str, entries: &[(&CheckResult, &str)]) {
    // Create the override directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(override_path).parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            println!(
                "    {}",
                format!("Failed to create directory {}: {e}", parent.display()).red()
            );
            println!("    {}", "Try running with sudo".yellow());
            return;
        }
    }

    // Read existing override file or start fresh
    let mut content = fs::read_to_string(override_path).unwrap_or_default();

    // Ensure [Service] section exists
    if !content.contains("[Service]") {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("[Service]\n");
    }

    // Add/update Environment= lines
    for (_r, env_line) in entries {
        // env_line is "Environment=GODEBUG=cgocheck=0"
        if let Some(key_val) = env_line.strip_prefix("Environment=") {
            if let Some((key, _)) = key_val.split_once('=') {
                let search = format!("Environment={key}=");
                let mut found = false;
                let new_content: Vec<String> = content
                    .lines()
                    .map(|l| {
                        if l.trim().starts_with(&search) {
                            found = true;
                            env_line.to_string()
                        } else {
                            l.to_string()
                        }
                    })
                    .collect();

                if found {
                    content = new_content.join("\n");
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                } else {
                    // Append after [Service]
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(env_line);
                    content.push('\n');
                }
            }
        }
    }

    match fs::write(override_path, &content) {
        Ok(_) => {
            println!(
                "    {}",
                format!("Written to {override_path}").green().bold()
            );

            // Run systemctl daemon-reload
            match Command::new("systemctl").arg("daemon-reload").output() {
                Ok(o) if o.status.success() => {
                    println!("    {}", "Ran systemctl daemon-reload.".green());
                }
                _ => {
                    println!(
                        "    {}",
                        "Could not run systemctl daemon-reload — run it manually.".yellow()
                    );
                }
            }

            println!(
                "    {}",
                "Restart FrankenPHP for changes to take effect.".yellow()
            );
        }
        Err(e) => {
            println!(
                "    {}",
                format!("Failed to write {override_path}: {e}").red()
            );
            println!("    {}", "Try running with sudo".yellow());
        }
    }
}

/// Full benchmark mode: benchmark all → apply all fixes → benchmark all → compare → keep/rollback.
/// Triggered by `--fix --bench` together.
pub fn propose_full_benchmark_fixes(
    results: &[CheckResult],
    frankenphp_bin: &str,
    app_path: &str,
    ctx: &SystemContext,
) {
    let fixable: Vec<_> = results
        .iter()
        .filter(|r| r.fix.is_some() && matches!(r.status, Status::Warn | Status::Fail))
        .collect();

    if fixable.is_empty() {
        println!();
        println!("  {}", "No fixes to apply.".dimmed());
        return;
    }

    // Classify fixes
    let mut file_fixes: BTreeMap<String, Vec<(&CheckResult, &str)>> = BTreeMap::new();
    let mut redis_fix_entries: Vec<(&CheckResult, &str)> = Vec::new();
    let mut systemd_fix_entries: BTreeMap<String, Vec<(&CheckResult, &str)>> = BTreeMap::new();
    let mut manual_fixes: Vec<(&CheckResult, &str)> = Vec::new();

    for r in &fixable {
        let fix = r.fix.as_ref().unwrap();
        if fix.file == "redis-cli" {
            redis_fix_entries.push((r, &fix.content));
        } else if fix.file.contains("systemd") && fix.file.ends_with("override.conf") {
            systemd_fix_entries
                .entry(fix.file.clone())
                .or_default()
                .push((r, &fix.content));
        } else if fix.file.starts_with('/') && !fix.file.contains('\u{2192}') {
            file_fixes
                .entry(fix.file.clone())
                .or_default()
                .push((r, &fix.content));
        } else {
            manual_fixes.push((r, &fix.content));
        }
    }

    let auto_count = file_fixes.values().map(|v| v.len()).sum::<usize>()
        + redis_fix_entries.len()
        + systemd_fix_entries.values().map(|v| v.len()).sum::<usize>();

    if auto_count == 0 {
        // Only manual fixes, show them
        if !manual_fixes.is_empty() {
            println!();
            println!("  {}", "Manual Actions Required".bold().underline());
            println!();
            for (r, content) in &manual_fixes {
                let fix = r.fix.as_ref().unwrap();
                println!(
                    "    {} {} ({})",
                    " WARN ".on_yellow().black().bold(),
                    r.label.bold(),
                    fix.description.dimmed()
                );
                println!("       {}", content.cyan());
                println!();
            }
        }
        return;
    }

    // Show all fixes that will be applied
    println!();
    println!("  {}", "Full Benchmark Mode".bold().underline());
    println!(
        "  {}",
        "All fixes will be applied, benchmarked, then you decide: keep or rollback.".dimmed()
    );
    println!();

    let mut fix_num = 0;
    for (file, entries) in &file_fixes {
        println!("    {}", file.bold());
        for (_r, content) in entries {
            fix_num += 1;
            println!(
                "      {}. {}",
                format!("{fix_num}").dimmed(),
                content.cyan()
            );
        }
        println!();
    }

    if !redis_fix_entries.is_empty() {
        println!("    {}", "Redis".bold());
        for (_r, content) in &redis_fix_entries {
            fix_num += 1;
            println!(
                "      {}. redis-cli CONFIG SET {}",
                format!("{fix_num}").dimmed(),
                content.cyan()
            );
        }
        println!();
    }

    for (file, entries) in &systemd_fix_entries {
        println!("    {}", file.bold());
        for (_r, content) in entries {
            fix_num += 1;
            println!(
                "      {}. {}",
                format!("{fix_num}").dimmed(),
                content.cyan()
            );
        }
        println!();
    }

    // Confirm before starting
    let start = dialoguer::Select::new()
        .with_prompt(format!("    Run full benchmark with {auto_count} fix(es)?"))
        .items(&["Start", "Cancel"])
        .default(0)
        .interact();

    if !matches!(start, Ok(0)) {
        println!("    {}", "Cancelled.".dimmed());
        return;
    }

    // === STEP 1: Benchmark BEFORE ===
    println!();
    println!(
        "  {} {}",
        "\u{25B6}".cyan(),
        "Running full benchmark (BEFORE fixes)...".bold()
    );
    println!();

    let php_before = benchmark::run(BenchmarkKind::Php, frankenphp_bin, app_path, 5);
    let mysql_before = if ctx.mysql_running {
        benchmark::run(BenchmarkKind::Mysql, frankenphp_bin, app_path, 5)
    } else {
        None
    };

    if let Some(ref b) = php_before {
        println!("    {}", "PHP Performance".bold());
        b.display_summary();
        println!();
    }
    if let Some(ref b) = mysql_before {
        println!("    {}", "MySQL Performance".bold());
        b.display_summary();
        println!();
    }

    // === STEP 2: Create backups & apply all fixes ===
    println!("  {} {}", "\u{25B6}".cyan(), "Applying all fixes...".bold());
    println!();

    // Backup files
    let mut file_backups: BTreeMap<String, Option<String>> = BTreeMap::new();
    for file in file_fixes.keys() {
        file_backups.insert(file.clone(), fs::read_to_string(file).ok());
    }

    // Backup Redis config
    let redis_backups: Vec<(String, String)> = redis_fix_entries
        .iter()
        .filter_map(|(_r, content)| {
            let key = content.split_whitespace().next()?;
            let val = redis_config_get(key)?;
            Some((key.to_string(), val))
        })
        .collect();

    // Backup systemd override files
    let mut systemd_backups: BTreeMap<String, Option<String>> = BTreeMap::new();
    for file in systemd_fix_entries.keys() {
        systemd_backups.insert(file.clone(), fs::read_to_string(file).ok());
    }

    // Apply file fixes
    let mut mysql_runtime_backups: Vec<(String, String)> = Vec::new();
    for (file, entries) in &file_fixes {
        apply_file_fixes_silent(file, entries);
        println!("    {} {file}", "Applied".green());

        // For MySQL CNF files, also apply changes at runtime via SET GLOBAL
        let is_mysql_cnf =
            file.ends_with(".cnf") && (file.contains("mysql") || file.contains("mariadb"));
        if is_mysql_cnf {
            let backups = apply_mysql_runtime_silent(entries);
            if !backups.is_empty() {
                println!(
                    "    {}",
                    format!(
                        "Applied {} variable(s) at runtime (SET GLOBAL)",
                        backups.len()
                    )
                    .green()
                    .to_string()
                );
            }
            mysql_runtime_backups.extend(backups);
        }
    }

    // Apply Redis fixes
    for (_r, content) in &redis_fix_entries {
        let parts: Vec<&str> = content.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let output = Command::new("redis-cli")
                .args(["CONFIG", "SET", parts[0], parts[1]])
                .output();
            if let Ok(o) = output {
                if o.status.success() {
                    println!("    {} redis {}", "Applied".green(), content);
                }
            }
        }
    }

    // Apply systemd fixes
    for (file, entries) in &systemd_fix_entries {
        apply_systemd_env_fixes_silent(file, entries);
        println!("    {} {file}", "Applied".green());
    }

    println!();

    // === STEP 3: Benchmark AFTER ===
    println!(
        "  {} {}",
        "\u{25B6}".cyan(),
        "Running full benchmark (AFTER fixes)...".bold()
    );
    println!();

    let php_after = benchmark::run(BenchmarkKind::Php, frankenphp_bin, app_path, 5);
    let mysql_after = if ctx.mysql_running {
        benchmark::run(BenchmarkKind::Mysql, frankenphp_bin, app_path, 5)
    } else {
        None
    };

    if let Some(ref a) = php_after {
        println!("    {}", "PHP Performance".bold());
        a.display_summary();
        println!();
    }
    if let Some(ref a) = mysql_after {
        println!("    {}", "MySQL Performance".bold());
        a.display_summary();
        println!();
    }

    // === STEP 4: Show comparison ===
    if let (Some(ref b), Some(ref a)) = (&php_before, &php_after) {
        benchmark::BenchmarkResult::display_comparison(b, a);
    }
    if let (Some(ref b), Some(ref a)) = (&mysql_before, &mysql_after) {
        benchmark::BenchmarkResult::display_comparison(b, a);
    }

    // === STEP 5: Keep or rollback? ===
    let keep = dialoguer::Select::new()
        .with_prompt("    Keep all applied fixes?")
        .items(&["Keep all fixes", "Rollback everything"])
        .default(0)
        .interact();

    match keep {
        Ok(0) => {
            println!();
            println!("    {}", "All fixes applied permanently.".green().bold());

            // Persist Redis changes
            if !redis_fix_entries.is_empty() {
                let _ = Command::new("redis-cli")
                    .args(["CONFIG", "REWRITE"])
                    .output();
                println!(
                    "    {}",
                    "Redis configuration persisted (CONFIG REWRITE).".green()
                );
            }

            // daemon-reload for systemd
            if !systemd_fix_entries.is_empty() {
                let _ = Command::new("systemctl").arg("daemon-reload").output();
                println!("    {}", "Ran systemctl daemon-reload.".green());
            }

            println!();
            if mysql_runtime_backups.is_empty() {
                println!(
                    "    {}",
                    "Restart FrankenPHP and MySQL for changes to take effect.".yellow()
                );
            } else {
                println!(
                    "    {}",
                    "MySQL runtime variables are already active. Restart FrankenPHP for remaining changes."
                        .yellow()
                );
            }
        }
        _ => {
            println!();
            println!(
                "    {} {}",
                "\u{25B6}".cyan(),
                "Rolling back all changes...".bold()
            );

            // Rollback file fixes
            for (file, backup) in &file_backups {
                match backup {
                    Some(content) => {
                        if fs::write(file, content).is_ok() {
                            println!("    {} {file}", "Restored".green());
                        } else {
                            println!("    {} {file}", "Failed to restore".red());
                        }
                    }
                    None => {
                        if fs::remove_file(file).is_ok() {
                            println!("    {} {file} (removed)", "Restored".green());
                        }
                    }
                }
            }

            // Rollback MySQL runtime variables
            if !mysql_runtime_backups.is_empty() {
                restore_mysql_runtime(&mysql_runtime_backups);
                println!(
                    "    {} MySQL runtime ({} variable(s))",
                    "Restored".green(),
                    mysql_runtime_backups.len()
                );
            }

            // Rollback Redis
            for (key, val) in &redis_backups {
                let _ = Command::new("redis-cli")
                    .args(["CONFIG", "SET", key, val])
                    .output();
                println!("    {} redis {} = {}", "Restored".green(), key, val);
            }

            // Rollback systemd overrides
            for (file, backup) in &systemd_backups {
                match backup {
                    Some(content) => {
                        if fs::write(file, content).is_ok() {
                            println!("    {} {file}", "Restored".green());
                        }
                    }
                    None => {
                        if fs::remove_file(file).is_ok() {
                            println!("    {} {file} (removed)", "Restored".green());
                        }
                    }
                }
            }

            if !systemd_fix_entries.is_empty() {
                let _ = Command::new("systemctl").arg("daemon-reload").output();
            }

            println!();
            println!("    {}", "All changes rolled back.".green().bold());
        }
    }

    // Show manual fixes at the end
    if !manual_fixes.is_empty() {
        println!();
        println!("    {}", "Manual Actions Required".bold());
        println!();
        for (r, content) in &manual_fixes {
            let fix = r.fix.as_ref().unwrap();
            println!(
                "      {} {} ({})",
                " WARN ".on_yellow().black().bold(),
                r.label.bold(),
                fix.description.dimmed()
            );
            println!("         {}", content.cyan());
            println!();
        }
    }
}

/// Apply systemd env fixes silently (no output, no daemon-reload). Used during full benchmark flow.
fn apply_systemd_env_fixes_silent(override_path: &str, entries: &[(&CheckResult, &str)]) {
    if let Some(parent) = std::path::Path::new(override_path).parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut content = fs::read_to_string(override_path).unwrap_or_default();

    if !content.contains("[Service]") {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("[Service]\n");
    }

    for (_r, env_line) in entries {
        if let Some(key_val) = env_line.strip_prefix("Environment=") {
            if let Some((key, _)) = key_val.split_once('=') {
                let search = format!("Environment={key}=");
                let mut found = false;
                let new_content: Vec<String> = content
                    .lines()
                    .map(|l| {
                        if l.trim().starts_with(&search) {
                            found = true;
                            env_line.to_string()
                        } else {
                            l.to_string()
                        }
                    })
                    .collect();

                if found {
                    content = new_content.join("\n");
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                } else {
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(env_line);
                    content.push('\n');
                }
            }
        }
    }

    let _ = fs::write(override_path, &content);
}

/// Read a Redis config value via redis-cli CONFIG GET.
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

fn print_restart_hint(file: &str) {
    if file.contains("php.ini") || file.contains("php-zts") {
        println!(
            "    {}",
            "Restart FrankenPHP for changes to take effect.".yellow()
        );
    } else if file.contains("mysql") || file.contains(".cnf") {
        println!(
            "    {}",
            "Restart MySQL for changes to take effect.".yellow()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CheckResult;
    use tempfile;

    fn dummy_result(fix_content: &str) -> CheckResult {
        CheckResult::warn("test", "test detail").with_fix("test fix", "/dummy", fix_content)
    }

    fn entries_from(results: &[CheckResult]) -> Vec<(&CheckResult, &str)> {
        results
            .iter()
            .map(|r| (r, r.fix.as_ref().unwrap().content.as_str()))
            .collect()
    }

    #[test]
    fn mysql_cnf_new_file_gets_mysqld_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-custom.cnf");
        let path_str = path.to_str().unwrap();

        let results = vec![dummy_result("innodb_buffer_pool_size=768M")];
        let entries = entries_from(&results);

        let count = write_fixes(path_str, &entries).unwrap();
        assert_eq!(count, 1);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("[mysqld]"),
            "Missing [mysqld] header in: {content}"
        );
        assert!(content.contains("innodb_buffer_pool_size=768M"));
    }

    #[test]
    fn mysql_cnf_existing_header_not_duplicated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-custom.cnf");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "[mysqld]\nmax_connections=50\n").unwrap();

        let results = vec![dummy_result("max_connections=200")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let header_count = content.matches("[mysqld]").count();
        assert_eq!(header_count, 1, "Should not duplicate [mysqld] header");
        assert!(content.contains("max_connections=200"));
        assert!(!content.contains("max_connections=50"));
    }

    #[test]
    fn mysql_cnf_appends_under_mysqld() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-custom.cnf");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "[mysqld]\ninnodb_log_file_size=64M\n").unwrap();

        let results = vec![dummy_result("tmp_table_size=64M")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[mysqld]"));
        assert!(content.contains("innodb_log_file_size=64M"));
        assert!(content.contains("tmp_table_size=64M"));
    }

    #[test]
    fn php_ini_no_mysqld_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php.ini");
        let path_str = path.to_str().unwrap();

        let results = vec![dummy_result("opcache.enable=1")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("[mysqld]"),
            "php.ini should not get [mysqld] header"
        );
        assert!(content.contains("opcache.enable=1"));
    }

    #[test]
    fn replaces_commented_out_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-custom.cnf");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "[mysqld]\n;innodb_buffer_pool_size=128M\n").unwrap();

        let results = vec![dummy_result("innodb_buffer_pool_size=768M")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("innodb_buffer_pool_size=768M"));
        assert!(!content.contains(";innodb_buffer_pool_size"));
    }

    #[test]
    fn multiple_fixes_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-custom.cnf");
        let path_str = path.to_str().unwrap();

        let results = vec![
            dummy_result("innodb_buffer_pool_size=768M"),
            dummy_result("innodb_log_file_size=256M"),
            dummy_result("max_connections=200"),
        ];
        let entries = entries_from(&results);

        let count = write_fixes(path_str, &entries).unwrap();
        assert_eq!(count, 3);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[mysqld]"));
        assert!(content.contains("innodb_buffer_pool_size=768M"));
        assert!(content.contains("innodb_log_file_size=256M"));
        assert!(content.contains("max_connections=200"));
    }

    #[test]
    fn mariadb_cnf_gets_mysqld_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mariadb-custom.cnf");
        let path_str = path.to_str().unwrap();

        let results = vec![dummy_result("innodb_buffer_pool_size=512M")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("[mysqld]"),
            "MariaDB .cnf should also get [mysqld] header"
        );
    }

    // --- Key replacement edge cases ---

    #[test]
    fn replaces_key_with_spaces_around_equals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php.ini");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "opcache.enable = 0\n").unwrap();

        let results = vec![dummy_result("opcache.enable=1")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("opcache.enable=1"));
        assert!(!content.contains("opcache.enable = 0"));
    }

    #[test]
    fn does_not_replace_substring_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-custom.cnf");
        let path_str = path.to_str().unwrap();

        // "max" should not match "max_connections"
        std::fs::write(&path, "[mysqld]\nmax_connections=100\n").unwrap();

        let results = vec![dummy_result("max=50")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        // max_connections should be untouched
        assert!(content.contains("max_connections=100"));
        // max=50 should be appended
        assert!(content.contains("max=50"));
    }

    #[test]
    fn replaces_commented_with_space() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php.ini");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "; opcache.jit_buffer_size=64M\n").unwrap();

        let results = vec![dummy_result("opcache.jit_buffer_size=128M")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("opcache.jit_buffer_size=128M"));
        assert!(!content.contains("; opcache"));
    }

    #[test]
    fn multiline_fix_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-custom.cnf");
        let path_str = path.to_str().unwrap();

        let results = vec![dummy_result("slow_query_log=1\nlong_query_time=1")];
        let entries = entries_from(&results);

        let count = write_fixes(path_str, &entries).unwrap();
        assert_eq!(count, 2, "Each line counts as one fix");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("slow_query_log=1"));
        assert!(content.contains("long_query_time=1"));
    }

    #[test]
    fn empty_file_gets_header_and_fix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-empty.cnf");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "").unwrap();

        let results = vec![dummy_result("innodb_buffer_pool_size=768M")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[mysqld]"));
        assert!(content.contains("innodb_buffer_pool_size=768M"));
    }

    #[test]
    fn file_with_only_mysqld_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mysql-bare.cnf");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "[mysqld]\n").unwrap();

        let results = vec![dummy_result("max_connections=200")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches("[mysqld]").count(), 1);
        assert!(content.contains("max_connections=200"));
    }

    #[test]
    fn non_mysql_cnf_no_header() {
        // A .cnf file that's NOT in a mysql/mariadb path
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("something.cnf");
        let path_str = path.to_str().unwrap();

        let results = vec![dummy_result("key=value")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("[mysqld]"),
            "Non-mysql .cnf should not get [mysqld] header"
        );
    }

    #[test]
    fn preserves_other_content_in_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php.ini");
        let path_str = path.to_str().unwrap();

        std::fs::write(
            &path,
            "[PHP]\nmax_execution_time=30\nmemory_limit=128M\ndate.timezone=UTC\n",
        )
        .unwrap();

        let results = vec![dummy_result("memory_limit=256M")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[PHP]"), "Should preserve [PHP] header");
        assert!(
            content.contains("max_execution_time=30"),
            "Untouched keys preserved"
        );
        assert!(
            content.contains("date.timezone=UTC"),
            "Untouched keys preserved"
        );
        assert!(content.contains("memory_limit=256M"), "Key replaced");
        assert!(!content.contains("memory_limit=128M"), "Old value gone");
    }

    #[test]
    fn fix_applied_count_accurate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php.ini");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "opcache.enable=0\nrealpath_cache_ttl=120\n").unwrap();

        let results = vec![
            dummy_result("opcache.enable=1"),
            dummy_result("realpath_cache_ttl=600"),
            dummy_result("opcache.jit_buffer_size=128M"),
        ];
        let entries = entries_from(&results);

        let count = write_fixes(path_str, &entries).unwrap();
        assert_eq!(count, 3, "2 replacements + 1 append = 3");
    }

    #[test]
    fn write_fixes_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php.ini");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "opcache.enable=0\n").unwrap();

        let results = vec![dummy_result("opcache.enable=1")];
        let entries = entries_from(&results);

        write_fixes(path_str, &entries).unwrap();
        let after_first = std::fs::read_to_string(&path).unwrap();

        write_fixes(path_str, &entries).unwrap();
        let after_second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(
            after_first, after_second,
            "Applying same fix twice should be idempotent"
        );
    }

    // --- BenchmarkKind::from_file ---

    #[test]
    fn benchmark_kind_php_zts_path() {
        assert!(matches!(
            BenchmarkKind::from_file("/etc/php-zts/php.ini"),
            BenchmarkKind::Php
        ));
    }

    #[test]
    fn benchmark_kind_mysql_conf_d() {
        assert!(matches!(
            BenchmarkKind::from_file("/etc/mysql/conf.d/custom.cnf"),
            BenchmarkKind::Mysql
        ));
    }

    #[test]
    fn benchmark_kind_env_file() {
        assert!(matches!(
            BenchmarkKind::from_file("/home/forge/app/.env"),
            BenchmarkKind::None
        ));
    }

    #[test]
    fn benchmark_kind_systemd_override() {
        assert!(matches!(
            BenchmarkKind::from_file("/etc/systemd/system/frankenphp.service.d/override.conf"),
            BenchmarkKind::None
        ));
    }

    // --- cnf_value_to_set_global tests ---

    #[test]
    fn cnf_value_gigabytes() {
        assert_eq!(
            cnf_value_to_set_global("2G"),
            (2u64 * 1024 * 1024 * 1024).to_string()
        );
    }

    #[test]
    fn cnf_value_megabytes() {
        assert_eq!(
            cnf_value_to_set_global("768M"),
            (768u64 * 1024 * 1024).to_string()
        );
    }

    #[test]
    fn cnf_value_kilobytes() {
        assert_eq!(cnf_value_to_set_global("512K"), (512u64 * 1024).to_string());
    }

    #[test]
    fn cnf_value_plain_number() {
        assert_eq!(cnf_value_to_set_global("200"), "200");
    }

    #[test]
    fn cnf_value_zero() {
        assert_eq!(cnf_value_to_set_global("0"), "0");
    }

    #[test]
    fn cnf_value_with_whitespace() {
        assert_eq!(
            cnf_value_to_set_global(" 256M "),
            (256u64 * 1024 * 1024).to_string()
        );
    }

    #[test]
    fn static_mysql_vars_contains_log_file_size() {
        assert!(STATIC_MYSQL_VARS.contains(&"innodb_log_file_size"));
    }
}

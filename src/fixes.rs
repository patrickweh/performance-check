use crate::benchmark::{self, BenchmarkKind};
use crate::types::{CheckResult, Status, SystemContext};
use colored::Colorize;
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

/// Interactively propose fixes for WARN/FAIL results that have fixable actions.
/// For auto-applicable fixes: runs a fix-specific benchmark before and after,
/// then asks the user whether to keep the fix or restore the original file.
pub fn propose_interactive_fixes(
    results: &[CheckResult],
    frankenphp_bin: &str,
    app_path: &str,
) {
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
            println!(
                "         {} {}",
                "\u{2192}".dimmed(),
                content.cyan()
            );
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
                    format!("Apply {} fix(es)? (benchmark: {})", entries.len(), kind.label())
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
                    apply_with_benchmark(file, &entries, frankenphp_bin, app_path, kind);
                }
                Ok(1) => {
                    apply_file_fixes(file, &entries);
                }
                _ => {
                    println!("    {}", "Skipped.".dimmed());
                }
            }
        } else {
            match selection {
                Ok(0) => {
                    apply_file_fixes(file, &entries);
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
            println!(
                "         {} {}",
                "\u{2192}".dimmed(),
                content.cyan()
            );
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
                apply_systemd_env_fixes(file, &entries);
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
    println!(
        "    {} Applying fix temporarily...",
        "\u{25B6}".cyan()
    );
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
                        println!(
                            "    {}",
                            format!("Restored original {file}").green()
                        );
                    }
                    Err(e) => {
                        println!(
                            "    {}",
                            format!("Failed to restore {file}: {e}").red()
                        );
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
                        println!(
                            "    {}",
                            format!("Failed to remove {file}: {e}").red()
                        );
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
            println!(
                "    {}",
                format!("Failed to write {file}: {e}").red()
            );
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
    let is_mysql_cnf = file.ends_with(".cnf") && (file.contains("mysql") || file.contains("mariadb"));
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
                        if trimmed.starts_with(key)
                            && trimmed[key.len()..].trim_start().starts_with('=')
                        {
                            found = true;
                            line.to_string()
                        } else if trimmed.starts_with(&format!(";{key}"))
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
                    println!(
                        "    {}",
                        format!("Set {} = {}", parts[0], parts[1]).green()
                    );
                } else {
                    println!(
                        "    {}",
                        format!("redis-cli CONFIG SET {} {}: {}", parts[0], parts[1], response).red()
                    );
                    any_failed = true;
                }
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                println!(
                    "    {}",
                    format!("Failed: redis-cli CONFIG SET {} {}: {}", parts[0], parts[1], err).red()
                );
                any_failed = true;
            }
            Err(e) => {
                println!(
                    "    {}",
                    format!("Could not run redis-cli: {e}").red()
                );
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
                    "Redis configuration persisted (CONFIG REWRITE).".green().bold()
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
                    println!(
                        "    {}",
                        "Ran systemctl daemon-reload.".green()
                    );
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
                println!("    {} {} ({})", " WARN ".on_yellow().black().bold(), r.label.bold(), fix.description.dimmed());
                println!("       {}", content.cyan());
                println!();
            }
        }
        return;
    }

    // Show all fixes that will be applied
    println!();
    println!("  {}", "Full Benchmark Mode".bold().underline());
    println!("  {}", "All fixes will be applied, benchmarked, then you decide: keep or rollback.".dimmed());
    println!();

    let mut fix_num = 0;
    for (file, entries) in &file_fixes {
        println!("    {}", file.bold());
        for (_r, content) in entries {
            fix_num += 1;
            println!("      {}. {}", format!("{fix_num}").dimmed(), content.cyan());
        }
        println!();
    }

    if !redis_fix_entries.is_empty() {
        println!("    {}", "Redis".bold());
        for (_r, content) in &redis_fix_entries {
            fix_num += 1;
            println!("      {}. redis-cli CONFIG SET {}", format!("{fix_num}").dimmed(), content.cyan());
        }
        println!();
    }

    for (file, entries) in &systemd_fix_entries {
        println!("    {}", file.bold());
        for (_r, content) in entries {
            fix_num += 1;
            println!("      {}. {}", format!("{fix_num}").dimmed(), content.cyan());
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
    println!(
        "  {} {}",
        "\u{25B6}".cyan(),
        "Applying all fixes...".bold()
    );
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
    for (file, entries) in &file_fixes {
        apply_file_fixes_silent(file, entries);
        println!("    {} {file}", "Applied".green());
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
                println!("    {}", "Redis configuration persisted (CONFIG REWRITE).".green());
            }

            // daemon-reload for systemd
            if !systemd_fix_entries.is_empty() {
                let _ = Command::new("systemctl").arg("daemon-reload").output();
                println!("    {}", "Ran systemctl daemon-reload.".green());
            }

            println!();
            println!("    {}", "Restart FrankenPHP and MySQL for changes to take effect.".yellow());
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

use crate::benchmark;
use crate::types::{CheckResult, Status};
use std::collections::BTreeMap;
use std::fs;

/// Interactively propose fixes for WARN/FAIL results that have fixable actions.
/// For auto-applicable fixes: runs a benchmark before and after, then asks the user
/// whether to keep the fix or restore the original file.
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
    println!("\x1b[1m═══ Interactive Fix Suggestions ═══\x1b[0m");
    println!();

    // Separate auto-applicable file fixes from manual fixes
    let mut auto_fixable: BTreeMap<String, Vec<(&CheckResult, &str)>> = BTreeMap::new();
    let mut manual_fixes: Vec<(&CheckResult, &str)> = Vec::new();

    for (file, entries) in &by_file {
        if file.starts_with('/') && !file.contains("→") {
            auto_fixable.insert(file.clone(), entries.clone());
        } else {
            for entry in entries {
                manual_fixes.push(*entry);
            }
        }
    }

    // Handle auto-fixable files with benchmark before/after
    for (file, entries) in &auto_fixable {
        println!("  \x1b[1m{file}\x1b[0m — {} fix(es) available:", entries.len());
        for (i, (r, content)) in entries.iter().enumerate() {
            println!(
                "    {}. [{}] {} → {}",
                i + 1,
                status_char(r.status),
                r.label,
                content
            );
        }
        println!();

        let selection = dialoguer::Select::new()
            .with_prompt(format!("  Try fixes for {file}? (will benchmark before/after)"))
            .items(&["Try with benchmark", "Apply without benchmark", "Skip"])
            .default(0)
            .interact();

        match selection {
            Ok(0) => {
                apply_with_benchmark(file, &entries, frankenphp_bin, app_path);
            }
            Ok(1) => {
                apply_file_fixes(file, &entries);
            }
            _ => {
                println!("  Skipped.");
            }
        }
        println!();
    }

    // Show manual fixes (Forge env, redis-cli commands)
    if !manual_fixes.is_empty() {
        println!("  \x1b[1mManual actions required:\x1b[0m");
        println!();
        for (r, content) in &manual_fixes {
            let fix = r.fix.as_ref().unwrap();
            println!(
                "    [{}] {} — {}:",
                status_char(r.status),
                r.label,
                fix.description
            );
            println!("    \x1b[36m{content}\x1b[0m");
            println!();
        }
    }
}

fn apply_with_benchmark(
    file: &str,
    entries: &[(&CheckResult, &str)],
    frankenphp_bin: &str,
    app_path: &str,
) {
    // 1. Create backup (full file content)
    let backup = match fs::read_to_string(file) {
        Ok(content) => content,
        Err(e) => {
            println!("  \x1b[31m✗ Cannot read {file} for backup: {e}\x1b[0m");
            return;
        }
    };

    // 2. Benchmark BEFORE
    println!("  \x1b[36m▶ Running benchmark (before fix)...\x1b[0m");
    let before = benchmark::run(frankenphp_bin, app_path, 3);

    if let Some(ref b) = before {
        println!(
            "    Cold start: {:.1}ms | PHP throughput: {:.1}ms",
            b.cold_start_ms, b.throughput_ms
        );
    } else {
        println!("  \x1b[33m⚠ Benchmark failed — applying fix without comparison\x1b[0m");
        apply_file_fixes(file, entries);
        return;
    }

    // 3. Apply fix temporarily
    println!("  \x1b[36m▶ Applying fix temporarily...\x1b[0m");
    apply_file_fixes_silent(file, entries);

    // 4. Benchmark AFTER
    println!("  \x1b[36m▶ Running benchmark (after fix)...\x1b[0m");
    let after = benchmark::run(frankenphp_bin, app_path, 3);

    if let Some(ref a) = after {
        println!(
            "    Cold start: {:.1}ms | PHP throughput: {:.1}ms",
            a.cold_start_ms, a.throughput_ms
        );
    }

    // 5. Show comparison
    if let (Some(ref b), Some(ref a)) = (&before, &after) {
        benchmark::BenchmarkResult::display_comparison(b, a);
    }

    // 6. Ask user: keep or revert?
    println!();
    let keep = dialoguer::Select::new()
        .with_prompt("  Keep the applied fix?")
        .items(&["Keep fix", "Revert to original"])
        .default(0)
        .interact();

    match keep {
        Ok(0) => {
            println!("  \x1b[32m✓ Fix kept\x1b[0m");
            print_restart_hint(file);
        }
        _ => {
            // Restore from backup — full file content, byte-for-byte
            match fs::write(file, &backup) {
                Ok(_) => {
                    println!("  \x1b[32m✓ Restored original {file}\x1b[0m");
                }
                Err(e) => {
                    println!("  \x1b[31m✗ Failed to restore {file}: {e}\x1b[0m");
                    println!("  \x1b[31m  Backup content was:\x1b[0m");
                    // Print first few lines so user can manually restore
                    for line in backup.lines().take(20) {
                        println!("  \x1b[31m  {line}\x1b[0m");
                    }
                }
            }
        }
    }
}

fn status_char(status: Status) -> &'static str {
    match status {
        Status::Fail => "FAIL",
        Status::Warn => "WARN",
        _ => "    ",
    }
}

/// Apply fixes and print status messages.
fn apply_file_fixes(file: &str, entries: &[(&CheckResult, &str)]) {
    let applied = write_fixes(file, entries);
    match applied {
        Ok(count) => {
            println!("  \x1b[32m✓ Applied {count} fix(es) to {file}\x1b[0m");
            print_restart_hint(file);
        }
        Err(e) => {
            println!("  \x1b[31m✗ Failed to write {file}: {e}\x1b[0m");
            println!("  \x1b[33m  Try running with sudo\x1b[0m");
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

fn print_restart_hint(file: &str) {
    if file.contains("php.ini") {
        println!("  \x1b[33m⚠ Restart FrankenPHP for changes to take effect\x1b[0m");
    } else if file.contains("mysql") || file.contains(".cnf") {
        println!("  \x1b[33m⚠ Restart MySQL for changes to take effect\x1b[0m");
    }
}

use crate::benchmark::{self, BenchmarkKind};
use crate::types::{CheckResult, Status};
use colored::Colorize;
use std::collections::BTreeMap;
use std::fs;

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

    // Separate auto-applicable file fixes from manual fixes
    let mut auto_fixable: BTreeMap<String, Vec<(&CheckResult, &str)>> = BTreeMap::new();
    let mut manual_fixes: Vec<(&CheckResult, &str)> = Vec::new();

    for (file, entries) in &by_file {
        if file.starts_with('/') && !file.contains('\u{2192}') {
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

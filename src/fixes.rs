use crate::types::{CheckResult, Status};
use std::collections::BTreeMap;
use std::fs;

/// Interactively propose fixes for WARN/FAIL results that have fixable actions.
/// Only modifies server config files (php.ini, mysql cnf, .env) — never touches the app repo.
pub fn propose_interactive_fixes(results: &[CheckResult]) {
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

    // Handle auto-fixable files (php.ini, mysql cnf, .env)
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
            .with_prompt(format!("  Apply fixes to {file}?"))
            .items(&["Apply all", "Skip"])
            .default(1)
            .interact();

        match selection {
            Ok(0) => {
                apply_file_fixes(file, entries);
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

fn status_char(status: Status) -> &'static str {
    match status {
        Status::Fail => "FAIL",
        Status::Warn => "WARN",
        _ => "    ",
    }
}

fn apply_file_fixes(file: &str, entries: &[(&CheckResult, &str)]) {
    let mut content = fs::read_to_string(file).unwrap_or_default();
    let mut applied = 0;

    for (_r, fix_line) in entries {
        // Each fix_line is like "key=value" or "key=value\nkey2=value2"
        for line in fix_line.lines() {
            if let Some((key, _value)) = line.split_once('=') {
                let key = key.trim();
                // Check if key already exists in file — replace the line
                let mut found = false;
                let new_content: Vec<String> = content
                    .lines()
                    .map(|l| {
                        let trimmed = l.trim();
                        // Match "key = ..." or ";key = ..." (commented out)
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
                    // Append
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

    match fs::write(file, &content) {
        Ok(_) => {
            println!("  \x1b[32m✓ Applied {applied} fix(es) to {file}\x1b[0m");
            if file.contains("php.ini") {
                println!("  \x1b[33m⚠ Restart FrankenPHP for changes to take effect\x1b[0m");
            } else if file.contains("mysql") || file.contains(".cnf") {
                println!("  \x1b[33m⚠ Restart MySQL for changes to take effect\x1b[0m");
            }
        }
        Err(e) => {
            println!("  \x1b[31m✗ Failed to write {file}: {e}\x1b[0m");
            println!("  \x1b[33m  Try running with sudo\x1b[0m");
        }
    }
}

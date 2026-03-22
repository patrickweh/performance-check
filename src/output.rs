use crate::types::{CheckResult, Status};
use colored::Colorize;
use std::collections::BTreeMap;

pub fn print_results(results: &[CheckResult], use_color: bool, json: bool) {
    if json {
        print_json(results);
        return;
    }

    if !use_color {
        colored::control::set_override(false);
    }

    println!();
    println!("{}", "═══ FrankenPHP + Laravel Performance Check ═══".bold());
    println!();

    for r in results {
        let prefix = match r.status {
            Status::Ok => "OK  ".green().bold(),
            Status::Warn => "WARN".yellow().bold(),
            Status::Fail => "FAIL".red().bold(),
            Status::Info => "INFO".blue().bold(),
        };

        if r.status == Status::Info {
            println!("{prefix} {}: {}", r.label.bold(), r.detail);
        } else {
            println!("{prefix} {}  ({})", r.label.bold(), r.detail);
        }
    }

    // Summary
    let (ok, warn, fail) = count_statuses(results);
    println!();
    println!("{}", "─".repeat(50));
    println!(
        "Pass: {}  Warn: {}  Fail: {}",
        format!("{ok}").green().bold(),
        format!("{warn}").yellow().bold(),
        format!("{fail}").red().bold(),
    );

    // Recommended values summary
    print_recommended_values(results, use_color);
}

fn count_statuses(results: &[CheckResult]) -> (usize, usize, usize) {
    let ok = results.iter().filter(|r| r.status == Status::Ok).count();
    let warn = results.iter().filter(|r| r.status == Status::Warn).count();
    let fail = results.iter().filter(|r| r.status == Status::Fail).count();
    (ok, warn, fail)
}

fn print_recommended_values(results: &[CheckResult], _use_color: bool) {
    let fixes: Vec<_> = results.iter().filter_map(|r| r.fix.as_ref()).collect();
    if fixes.is_empty() {
        return;
    }

    // Group by file
    let mut by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for fix in &fixes {
        by_file.entry(&fix.file).or_default().push(&fix.content);
    }

    println!();
    println!("{}", "═══ Recommended Values Summary ═══".bold());

    for (file, values) in &by_file {
        println!();
        println!("{}:", file.bold().underline());
        for val in values {
            println!("  {val}");
        }
    }

    println!();
}

fn print_json(results: &[CheckResult]) {
    let output = serde_json::json!({
        "results": results,
        "summary": {
            "pass": results.iter().filter(|r| r.status == Status::Ok).count(),
            "warn": results.iter().filter(|r| r.status == Status::Warn).count(),
            "fail": results.iter().filter(|r| r.status == Status::Fail).count(),
        }
    });
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

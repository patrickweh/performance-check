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

    // Header
    println!();
    println!(
        "  {}",
        "FrankenPHP + Laravel Performance Check".bold().underline()
    );
    println!();

    // Group results by section
    let sections = group_by_section(results);

    for (section_name, section_results) in &sections {
        println!("  {}", section_name.dimmed().bold());
        println!();

        for r in section_results {
            let (icon, colored_icon) = match r.status {
                Status::Ok => ("  ", "  ".green()),
                Status::Warn => ("  ", "  ".yellow()),
                Status::Fail => ("  ", "  ".red()),
                Status::Info => ("  ", "  ".blue()),
            };

            let _ = icon; // suppress unused warning
            let status_badge = match r.status {
                Status::Ok => " PASS ".on_green().white().bold(),
                Status::Warn => " WARN ".on_yellow().black().bold(),
                Status::Fail => " FAIL ".on_red().white().bold(),
                Status::Info => " INFO ".on_blue().white().bold(),
            };

            if r.status == Status::Info {
                println!(
                    "  {} {}{} {}",
                    colored_icon,
                    status_badge,
                    format!(" {}", r.label).bold(),
                    format!(" {}", r.detail).dimmed()
                );
            } else {
                println!(
                    "  {} {} {}",
                    colored_icon,
                    status_badge,
                    format!(" {}", r.label).bold(),
                );
                if !r.detail.is_empty() {
                    println!("          {}", r.detail.dimmed());
                }
            }
        }
        println!();
    }

    // Summary bar
    let (ok, warn, fail) = count_statuses(results);
    let total = results.len();

    println!("  {}", "Results".bold().underline());
    println!();

    let pass_bar = make_bar(ok, total, "green");
    let warn_bar = make_bar(warn, total, "yellow");
    let fail_bar = make_bar(fail, total, "red");

    println!(
        "    {} Pass: {}  {} Warn: {}  {} Fail: {}",
        pass_bar,
        format!("{ok}").green().bold(),
        warn_bar,
        format!("{warn}").yellow().bold(),
        fail_bar,
        format!("{fail}").red().bold(),
    );
    println!();

    // Score
    let score = if total > 0 {
        ((ok as f64 / (ok + warn + fail).max(1) as f64) * 100.0) as u32
    } else {
        100
    };

    let score_color = if score >= 80 {
        format!("{score}%").green().bold()
    } else if score >= 50 {
        format!("{score}%").yellow().bold()
    } else {
        format!("{score}%").red().bold()
    };

    println!("    Score: {score_color}");
    println!();

    // Recommended values summary
    print_recommended_values(results);
}

fn group_by_section(results: &[CheckResult]) -> Vec<(&'static str, Vec<&CheckResult>)> {
    let mut sections: Vec<(&str, Vec<&CheckResult>)> = Vec::new();

    let section_map: &[(&[&str], &str)] = &[
        (
            &[
                "CPU Cores",
                "Memory",
                "Swap Usage",
                "PHP RAM Budget",
                "Laravel Version",
                "MySQL/MariaDB",
                "Redis",
            ],
            "System",
        ),
        (&["libc"], "Runtime"),
        (&["FrankenPHP Binary", "FrankenPHP Version"], "FrankenPHP"),
        (&["PHP-ZTS", "PHP ext:"], "PHP Extensions"),
        (
            &[
                "opcache.",
                "realpath_cache",
                "memory_limit",
                "Worker Memory",
            ],
            "PHP Configuration",
        ),
        (&["GODEBUG", "GOMEMLIMIT"], "Go Runtime"),
        (
            &[
                "APP_ENV",
                "APP_DEBUG",
                "OCTANE_HTTPS",
                "CACHE_STORE",
                "QUEUE_CONNECTION",
                "SESSION_DRIVER",
                "LOG_CHANNEL",
            ],
            "Laravel Environment",
        ),
        (&["Bootstrap Cache", "composer"], "Laravel Application"),
        (
            &[
                "MySQL Version",
                "innodb_",
                "query_cache",
                "slow_query",
                "long_query",
                "max_connections",
                "tmp_table",
            ],
            "MySQL / MariaDB",
        ),
        (&["Redis"], "Redis"),
    ];

    let mut assigned: Vec<bool> = vec![false; results.len()];

    for (prefixes, section_name) in section_map {
        let mut section_results = Vec::new();
        for (i, r) in results.iter().enumerate() {
            if assigned[i] {
                continue;
            }
            if prefixes.iter().any(|p| r.label.starts_with(p)) {
                section_results.push(r);
                assigned[i] = true;
            }
        }
        if !section_results.is_empty() {
            sections.push((section_name, section_results));
        }
    }

    // Collect unassigned into "Other"
    let other: Vec<&CheckResult> = results
        .iter()
        .enumerate()
        .filter(|(i, _)| !assigned[*i])
        .map(|(_, r)| r)
        .collect();
    if !other.is_empty() {
        sections.push(("Other", other));
    }

    sections
}

fn count_statuses(results: &[CheckResult]) -> (usize, usize, usize) {
    let ok = results.iter().filter(|r| r.status == Status::Ok).count();
    let warn = results.iter().filter(|r| r.status == Status::Warn).count();
    let fail = results.iter().filter(|r| r.status == Status::Fail).count();
    (ok, warn, fail)
}

fn make_bar(count: usize, total: usize, color: &str) -> String {
    let width = 8;
    let filled = if total > 0 {
        (count * width / total).max(if count > 0 { 1 } else { 0 })
    } else {
        0
    };
    let bar: String = std::iter::repeat_n('█', filled)
        .chain(std::iter::repeat_n('░', width - filled))
        .collect();
    match color {
        "green" => format!("{}", bar.green()),
        "yellow" => format!("{}", bar.yellow()),
        "red" => format!("{}", bar.red()),
        _ => bar,
    }
}

fn print_recommended_values(results: &[CheckResult]) {
    let fixes: Vec<_> = results.iter().filter_map(|r| r.fix.as_ref()).collect();
    if fixes.is_empty() {
        return;
    }

    // Group by file
    let mut by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for fix in &fixes {
        by_file.entry(&fix.file).or_default().push(&fix.content);
    }

    println!("  {}", "Recommended Changes".bold().underline());
    println!();

    for (file, values) in &by_file {
        println!("    {}", file.bold());
        for val in values {
            println!("      {}", val.cyan());
        }
        println!();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CheckResult;

    #[test]
    fn count_statuses_all_ok() {
        let results = vec![CheckResult::ok("a", ""), CheckResult::ok("b", "")];
        assert_eq!(count_statuses(&results), (2, 0, 0));
    }

    #[test]
    fn count_statuses_mixed() {
        let results = vec![
            CheckResult::ok("a", ""),
            CheckResult::warn("b", ""),
            CheckResult::fail("c", ""),
            CheckResult::info("d", ""),
        ];
        // Info is not counted in ok/warn/fail
        assert_eq!(count_statuses(&results), (1, 1, 1));
    }

    #[test]
    fn count_statuses_empty() {
        assert_eq!(count_statuses(&[]), (0, 0, 0));
    }

    #[test]
    fn group_by_section_system() {
        let results = vec![
            CheckResult::info("CPU Cores", "4"),
            CheckResult::info("Memory", "8192MB"),
            CheckResult::ok("Swap Usage", "No swap"),
        ];
        let sections = group_by_section(&results);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "System");
        assert_eq!(sections[0].1.len(), 3);
    }

    #[test]
    fn group_by_section_multiple() {
        let results = vec![
            CheckResult::info("CPU Cores", "4"),
            CheckResult::ok("opcache.enable", "On"),
            CheckResult::ok("innodb_buffer_pool_size", "768M"),
        ];
        let sections = group_by_section(&results);
        assert!(sections.len() >= 3);
        let section_names: Vec<&str> = sections.iter().map(|(n, _)| *n).collect();
        assert!(section_names.contains(&"System"));
        assert!(section_names.contains(&"PHP Configuration"));
        assert!(section_names.contains(&"MySQL / MariaDB"));
    }

    #[test]
    fn group_by_section_unassigned_goes_to_other() {
        let results = vec![CheckResult::ok("SomethingUnknown", "value")];
        let sections = group_by_section(&results);
        assert_eq!(sections[0].0, "Other");
    }

    #[test]
    fn group_by_section_laravel_env() {
        let results = vec![
            CheckResult::ok("APP_ENV", "production"),
            CheckResult::fail("APP_DEBUG", "true"),
            CheckResult::ok("QUEUE_CONNECTION", "redis"),
        ];
        let sections = group_by_section(&results);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "Laravel Environment");
        assert_eq!(sections[0].1.len(), 3);
    }

    #[test]
    fn make_bar_full() {
        // When all results are one status, that bar should be full
        let bar = make_bar(10, 10, "green");
        assert_eq!(bar.matches('█').count(), 8); // width=8
    }

    #[test]
    fn make_bar_empty() {
        let bar = make_bar(0, 10, "red");
        assert_eq!(bar.matches('░').count(), 8);
    }

    #[test]
    fn make_bar_zero_total() {
        let bar = make_bar(0, 0, "yellow");
        assert_eq!(bar.matches('░').count(), 8);
    }
}

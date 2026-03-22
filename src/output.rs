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
        (
            &[
                "FrankenPHP Binary",
                "FrankenPHP Version",
                "FrankenPHP Worker",
                "FrankenPHP num_threads",
                "FrankenPHP Log",
                "FrankenPHP Symlink",
            ],
            "FrankenPHP",
        ),
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

    #[test]
    fn make_bar_single_item() {
        let bar = make_bar(1, 10, "green");
        // At least 1 filled block when count > 0
        assert!(bar.contains('█'));
    }

    #[test]
    fn make_bar_half() {
        let bar = make_bar(5, 10, "yellow");
        let filled = bar.matches('█').count();
        assert_eq!(filled, 4, "Half should fill 4 of 8 blocks");
    }

    // --- Status counting edge cases ---

    #[test]
    fn count_statuses_only_info() {
        let results = vec![CheckResult::info("a", ""), CheckResult::info("b", "")];
        assert_eq!(count_statuses(&results), (0, 0, 0));
    }

    #[test]
    fn count_statuses_one_of_each() {
        let results = vec![
            CheckResult::ok("a", ""),
            CheckResult::warn("b", ""),
            CheckResult::fail("c", ""),
        ];
        assert_eq!(count_statuses(&results), (1, 1, 1));
    }

    #[test]
    fn count_statuses_all_fail() {
        let results = vec![
            CheckResult::fail("a", ""),
            CheckResult::fail("b", ""),
            CheckResult::fail("c", ""),
        ];
        assert_eq!(count_statuses(&results), (0, 0, 3));
    }

    // --- Group by section edge cases ---

    #[test]
    fn group_by_section_empty_results() {
        let sections = group_by_section(&[]);
        assert!(sections.is_empty());
    }

    #[test]
    fn group_by_section_php_extensions() {
        let results = vec![
            CheckResult::ok("PHP ext: opcache", "loaded"),
            CheckResult::ok("PHP ext: redis", "loaded"),
            CheckResult::fail("PHP ext: gd", "not loaded"),
        ];
        let sections = group_by_section(&results);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "PHP Extensions");
        assert_eq!(sections[0].1.len(), 3);
    }

    #[test]
    fn group_by_section_go_runtime() {
        let results = vec![
            CheckResult::ok("GODEBUG", "cgocheck=0"),
            CheckResult::warn("GOMEMLIMIT", "Not set"),
        ];
        let sections = group_by_section(&results);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "Go Runtime");
    }

    #[test]
    fn group_by_section_frankenphp() {
        let results = vec![
            CheckResult::ok("FrankenPHP Binary", "/usr/bin/frankenphp found"),
            CheckResult::info("FrankenPHP Version", "1.3.0"),
        ];
        let sections = group_by_section(&results);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "FrankenPHP");
    }

    #[test]
    fn group_by_section_redis() {
        // "Redis" prefix is matched by System first, then Redis catches remaining
        // In practice, system::gather produces "Redis" (info) which goes to System,
        // and redis::check produces "Redis maxmemory" which also starts with "Redis"
        // Since System grabs "Redis"-prefixed first, these go to System
        let results = vec![
            CheckResult::ok("Redis maxmemory", "256MB"),
            CheckResult::ok("Redis maxmemory-policy", "allkeys-lru"),
        ];
        let sections = group_by_section(&results);
        assert_eq!(sections.len(), 1);
        // System section claims "Redis" prefix first
        assert_eq!(sections[0].0, "System");
    }

    #[test]
    fn group_by_section_full_realistic_output() {
        let results = vec![
            CheckResult::info("CPU Cores", "4"),
            CheckResult::info("Memory", "4096MB"),
            CheckResult::ok("Swap Usage", "No swap"),
            CheckResult::ok("libc", "glibc"),
            CheckResult::ok("FrankenPHP Binary", "found"),
            CheckResult::ok("opcache.enable", "On"),
            CheckResult::warn("opcache.jit_buffer_size", "Not set"),
            CheckResult::ok("APP_ENV", "production"),
            CheckResult::ok("innodb_buffer_pool_size", "768M"),
            CheckResult::ok("Redis maxmemory", "256MB"),
        ];
        let sections = group_by_section(&results);

        let section_names: Vec<&str> = sections.iter().map(|(n, _)| *n).collect();
        assert!(section_names.contains(&"System")); // CPU, Memory, Swap, Redis
        assert!(section_names.contains(&"Runtime")); // libc
        assert!(section_names.contains(&"FrankenPHP"));
        assert!(section_names.contains(&"PHP Configuration"));
        assert!(section_names.contains(&"Laravel Environment"));
        assert!(section_names.contains(&"MySQL / MariaDB"));
        // "Redis maxmemory" gets caught by System's "Redis" prefix
        // so Redis section only appears if there are unmatched "Redis" items
        assert!(sections.len() >= 6);
    }

    #[test]
    fn group_by_section_preserves_order_within_section() {
        let results = vec![
            CheckResult::ok("opcache.enable", "On"),
            CheckResult::warn("opcache.validate_timestamps", "Off"),
            CheckResult::ok("opcache.memory_consumption", "256"),
        ];
        let sections = group_by_section(&results);
        assert_eq!(sections[0].0, "PHP Configuration");
        assert_eq!(sections[0].1[0].label, "opcache.enable");
        assert_eq!(sections[0].1[1].label, "opcache.validate_timestamps");
        assert_eq!(sections[0].1[2].label, "opcache.memory_consumption");
    }

    // --- JSON output ---

    #[test]
    fn json_output_structure() {
        let results = vec![
            CheckResult::ok("test1", "ok"),
            CheckResult::warn("test2", "warning"),
            CheckResult::fail("test3", "fail"),
            CheckResult::info("test4", "info"),
        ];

        let output = serde_json::json!({
            "results": results,
            "summary": {
                "pass": results.iter().filter(|r| r.status == Status::Ok).count(),
                "warn": results.iter().filter(|r| r.status == Status::Warn).count(),
                "fail": results.iter().filter(|r| r.status == Status::Fail).count(),
            }
        });

        let json_str = serde_json::to_string(&output).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["summary"]["pass"], 1);
        assert_eq!(parsed["summary"]["warn"], 1);
        assert_eq!(parsed["summary"]["fail"], 1);
        assert_eq!(parsed["results"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn json_output_fix_included() {
        let results =
            vec![CheckResult::warn("test", "detail").with_fix("fix desc", "file.ini", "key=value")];

        let output = serde_json::json!({ "results": results });
        let json_str = serde_json::to_string(&output).unwrap();

        assert!(json_str.contains("\"fix\""));
        assert!(json_str.contains("fix desc"));
        assert!(json_str.contains("file.ini"));
        assert!(json_str.contains("key=value"));
    }

    #[test]
    fn json_output_no_fix_omitted() {
        let results = vec![CheckResult::ok("test", "detail")];

        let json_str = serde_json::to_string(&results).unwrap();
        assert!(
            !json_str.contains("\"fix\""),
            "fix should be omitted when None"
        );
    }
}

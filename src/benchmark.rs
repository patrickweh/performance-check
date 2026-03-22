use crate::types::SystemContext;
use colored::Colorize;
use std::process::Command;
use std::time::{Duration, Instant};

/// What kind of fix we're benchmarking, determines which benchmark to run.
#[derive(Debug, Clone, Copy)]
pub enum BenchmarkKind {
    /// PHP config changes (OPcache, realpath, memory) — measure PHP bootstrap + throughput
    Php,
    /// MySQL config changes — measure DB query performance
    Mysql,
    /// No meaningful benchmark possible (e.g. .env LOG_CHANNEL)
    None,
}

impl BenchmarkKind {
    /// Determine the benchmark kind from the target file path.
    pub fn from_file(file: &str) -> Self {
        if file.contains("php.ini") || file.contains("php-zts") {
            BenchmarkKind::Php
        } else if file.contains("mysql") || file.contains(".cnf") {
            BenchmarkKind::Mysql
        } else {
            BenchmarkKind::None
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            BenchmarkKind::Php => "PHP Performance",
            BenchmarkKind::Mysql => "MySQL Performance",
            BenchmarkKind::None => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub kind: &'static str,
    pub metrics: Vec<Metric>,
}

#[derive(Debug, Clone)]
pub struct Metric {
    pub label: String,
    pub value_ms: f64,
}

impl BenchmarkResult {
    pub fn display_summary(&self) {
        for m in &self.metrics {
            println!("      {} {:.1}ms", m.label.dimmed(), m.value_ms);
        }
    }

    pub fn display_comparison(before: &BenchmarkResult, after: &BenchmarkResult) {
        println!();
        println!(
            "    {}",
            format!("Benchmark: {}", before.kind).bold().underline()
        );
        println!();
        println!(
            "    {:<28} {:>10} {:>10}   {}",
            "Metric".dimmed(),
            "Before".dimmed(),
            "After".dimmed(),
            "Change".dimmed()
        );
        println!("    {}", "─".repeat(68));

        for (b, a) in before.metrics.iter().zip(after.metrics.iter()) {
            let diff = a.value_ms - b.value_ms;
            let pct = if b.value_ms > 0.0 {
                ((a.value_ms - b.value_ms) / b.value_ms) * 100.0
            } else {
                0.0
            };

            let change = if diff.abs() < 0.5 {
                "~0%".dimmed().to_string()
            } else if diff < 0.0 {
                format!("{:+.0}ms ({:+.1}%)", diff, pct)
                    .green()
                    .bold()
                    .to_string()
            } else {
                format!("+{:.0}ms (+{:.1}%)", diff, pct)
                    .red()
                    .bold()
                    .to_string()
            };

            println!(
                "    {:<28} {:>8.1}ms {:>8.1}ms   {}",
                b.label, b.value_ms, a.value_ms, change
            );
        }
        println!();
    }
}

/// Run all available benchmarks standalone (--bench flag).
pub fn run_all(frankenphp_bin: &str, app_path: &str, ctx: &SystemContext) {
    println!();
    println!("  {}", "Benchmarks".bold().underline());
    println!();

    // PHP benchmark
    println!("    {}", "PHP Performance".bold());
    println!("    {}", "Running 5 iterations...".dimmed());
    println!();

    match run_php_benchmark(frankenphp_bin, app_path, 5) {
        Some(result) => {
            for m in &result.metrics {
                println!("      {:<28} {:>8.1}ms", m.label, m.value_ms);
            }
        }
        None => {
            println!(
                "      {}",
                "Could not run PHP benchmark (is FrankenPHP available?)".yellow()
            );
        }
    }
    println!();

    // MySQL benchmark
    if ctx.mysql_running {
        println!("    {}", "MySQL Performance".bold());
        println!("    {}", "Running 5 iterations...".dimmed());
        println!();

        match run_mysql_benchmark(5) {
            Some(result) => {
                for m in &result.metrics {
                    if m.value_ms > 0.0 {
                        println!("      {:<28} {:>8.1}ms", m.label, m.value_ms);
                    } else {
                        // Non-timing metric (like buffer pool hit rate)
                        println!("      {}", m.label);
                    }
                }
            }
            None => {
                println!("      {}", "Could not run MySQL benchmark".yellow());
            }
        }
        println!();
    }
}

/// Run the appropriate benchmark based on the fix type.
pub fn run(
    kind: BenchmarkKind,
    frankenphp_bin: &str,
    app_path: &str,
    iterations: u32,
) -> Option<BenchmarkResult> {
    match kind {
        BenchmarkKind::Php => run_php_benchmark(frankenphp_bin, app_path, iterations),
        BenchmarkKind::Mysql => run_mysql_benchmark(iterations),
        BenchmarkKind::None => None,
    }
}

fn run_php_benchmark(
    frankenphp_bin: &str,
    app_path: &str,
    iterations: u32,
) -> Option<BenchmarkResult> {
    let cold_start = measure_cold_start(frankenphp_bin, app_path, iterations)?;
    let throughput = measure_php_throughput(frankenphp_bin, iterations);

    Some(BenchmarkResult {
        kind: "PHP Performance",
        metrics: vec![
            Metric {
                label: "Laravel Bootstrap".to_string(),
                value_ms: cold_start,
            },
            Metric {
                label: "PHP Compute (sqrt+md5)".to_string(),
                value_ms: throughput,
            },
        ],
    })
}

fn run_mysql_benchmark(iterations: u32) -> Option<BenchmarkResult> {
    let select = measure_mysql_query("SELECT BENCHMARK(100000, MD5('benchmark'))", iterations)?;

    let write = measure_mysql_query("DO BENCHMARK(100000, CRC32('benchmark'))", iterations);

    let mut metrics = vec![Metric {
        label: "SELECT throughput".to_string(),
        value_ms: select,
    }];

    if let Some(w) = write {
        metrics.push(Metric {
            label: "Compute throughput".to_string(),
            value_ms: w,
        });
    }

    // InnoDB buffer pool hit rate
    if let Some(hit_rate) = get_buffer_pool_hit_rate() {
        metrics.push(Metric {
            label: format!("Buffer pool hit rate: {:.1}%", hit_rate),
            value_ms: 0.0,
        });
    }

    Some(BenchmarkResult {
        kind: "MySQL Performance",
        metrics,
    })
}

fn measure_cold_start(frankenphp_bin: &str, app_path: &str, iterations: u32) -> Option<f64> {
    let mut times = Vec::new();

    for _ in 0..iterations {
        let start = Instant::now();
        let output = Command::new(frankenphp_bin)
            .args(["php-cli", "artisan", "--version"])
            .current_dir(app_path)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        times.push(start.elapsed());
    }

    Some(avg_ms(&times))
}

fn measure_php_throughput(frankenphp_bin: &str, iterations: u32) -> f64 {
    let code = r#"
        $start = hrtime(true);
        $sum = 0;
        for ($i = 0; $i < 100000; $i++) { $sum += sqrt($i); }
        $s = str_repeat('x', 10000);
        for ($i = 0; $i < 1000; $i++) { $s = md5($s); }
        echo (hrtime(true) - $start) / 1e6;
    "#;

    let mut times = Vec::new();

    for _ in 0..iterations {
        if let Some(ms) = php_eval_float(frankenphp_bin, code) {
            times.push(Duration::from_secs_f64(ms / 1000.0));
        }
    }

    if times.is_empty() {
        return 0.0;
    }

    avg_ms(&times)
}

fn measure_mysql_query(query: &str, iterations: u32) -> Option<f64> {
    let mut times = Vec::new();

    for _ in 0..iterations {
        let start = Instant::now();
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
            times.push(start.elapsed());
        }
    }

    if times.is_empty() {
        return None;
    }

    Some(avg_ms(&times))
}

fn get_buffer_pool_hit_rate() -> Option<f64> {
    let output = Command::new("mysql")
        .args([
            "--defaults-file=/etc/mysql/debian.cnf",
            "-N",
            "-B",
            "-e",
            "SELECT (1 - (Innodb_buffer_pool_reads / NULLIF(Innodb_buffer_pool_read_requests, 0))) * 100 FROM (SELECT VARIABLE_VALUE AS Innodb_buffer_pool_reads FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Innodb_buffer_pool_reads') a, (SELECT VARIABLE_VALUE AS Innodb_buffer_pool_read_requests FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Innodb_buffer_pool_read_requests') b",
        ])
        .output()
        .or_else(|_| {
            Command::new("mysql")
                .args([
                    "-u", "root", "-N", "-B", "-e",
                    "SELECT (1 - (Innodb_buffer_pool_reads / NULLIF(Innodb_buffer_pool_read_requests, 0))) * 100 FROM (SELECT VARIABLE_VALUE AS Innodb_buffer_pool_reads FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Innodb_buffer_pool_reads') a, (SELECT VARIABLE_VALUE AS Innodb_buffer_pool_read_requests FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Innodb_buffer_pool_read_requests') b",
                ])
                .output()
        })
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    text.parse::<f64>().ok()
}

fn php_eval_float(frankenphp_bin: &str, code: &str) -> Option<f64> {
    let output = Command::new(frankenphp_bin)
        .args(["php-cli", "-r", code])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    stdout.parse::<f64>().ok()
}

fn avg_ms(durations: &[Duration]) -> f64 {
    if durations.is_empty() {
        return 0.0;
    }
    let total: f64 = durations.iter().map(|d| d.as_secs_f64() * 1000.0).sum();
    total / durations.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn benchmark_kind_from_php_ini() {
        assert!(matches!(
            BenchmarkKind::from_file("/etc/php-zts/php.ini"),
            BenchmarkKind::Php
        ));
        assert!(matches!(
            BenchmarkKind::from_file("/etc/php.ini"),
            BenchmarkKind::Php
        ));
    }

    #[test]
    fn benchmark_kind_from_mysql_cnf() {
        assert!(matches!(
            BenchmarkKind::from_file("/etc/mysql/conf.d/custom.cnf"),
            BenchmarkKind::Mysql
        ));
    }

    #[test]
    fn benchmark_kind_from_env() {
        assert!(matches!(
            BenchmarkKind::from_file("/home/forge/app/.env"),
            BenchmarkKind::None
        ));
    }

    #[test]
    fn benchmark_kind_labels() {
        assert_eq!(BenchmarkKind::Php.label(), "PHP Performance");
        assert_eq!(BenchmarkKind::Mysql.label(), "MySQL Performance");
        assert_eq!(BenchmarkKind::None.label(), "");
    }

    #[test]
    fn avg_ms_single() {
        let durations = vec![Duration::from_millis(100)];
        let result = avg_ms(&durations);
        assert!((result - 100.0).abs() < 0.1);
    }

    #[test]
    fn avg_ms_multiple() {
        let durations = vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ];
        let result = avg_ms(&durations);
        assert!((result - 200.0).abs() < 0.1);
    }

    #[test]
    fn avg_ms_empty() {
        assert_eq!(avg_ms(&[]), 0.0);
    }

    #[test]
    fn benchmark_result_metrics() {
        let result = BenchmarkResult {
            kind: "Test",
            metrics: vec![
                Metric {
                    label: "Cold Start".to_string(),
                    value_ms: 150.0,
                },
                Metric {
                    label: "Throughput".to_string(),
                    value_ms: 45.5,
                },
            ],
        };
        assert_eq!(result.metrics.len(), 2);
        assert_eq!(result.metrics[0].label, "Cold Start");
        assert!((result.metrics[0].value_ms - 150.0).abs() < f64::EPSILON);
    }
}

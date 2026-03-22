use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BenchmarkResult {
    /// Average Laravel bootstrap time via CLI (ms)
    pub cold_start_ms: f64,
    /// Average PHP computation throughput time (ms)
    pub throughput_ms: f64,
    /// Number of iterations run
    pub iterations: u32,
}

impl BenchmarkResult {
    pub fn display_comparison(before: &BenchmarkResult, after: &BenchmarkResult) {
        println!();
        println!("  \x1b[1m── Benchmark Comparison ──\x1b[0m");
        println!();
        println!(
            "  {:<24} {:>10} {:>10} {:>10}",
            "", "Before", "After", "Change"
        );
        println!("  {}", "─".repeat(56));

        Self::print_row("Cold Start (bootstrap)", before.cold_start_ms, after.cold_start_ms);
        Self::print_row("PHP Throughput", before.throughput_ms, after.throughput_ms);
    }

    fn print_row(label: &str, before: f64, after: f64) {
        let diff = after - before;
        let pct = if before > 0.0 {
            ((after - before) / before) * 100.0
        } else {
            0.0
        };

        let change = if diff.abs() < 0.5 {
            format!("  ~0%")
        } else if diff < 0.0 {
            format!("\x1b[32m{:+.0}ms ({:+.1}%)\x1b[0m", diff, pct)
        } else {
            format!("\x1b[31m+{:.0}ms (+{:.1}%)\x1b[0m", diff, pct)
        };

        println!(
            "  {:<24} {:>8.1}ms {:>8.1}ms  {}",
            label, before, after, change
        );
    }
}

/// Run a benchmark using frankenphp php-cli.
/// Measures cold start (Laravel bootstrap) and PHP computation throughput.
pub fn run(frankenphp_bin: &str, app_path: &str, iterations: u32) -> Option<BenchmarkResult> {
    let cold_start = measure_cold_start(frankenphp_bin, app_path, iterations)?;
    let throughput = measure_throughput(frankenphp_bin, iterations);

    Some(BenchmarkResult {
        cold_start_ms: cold_start,
        throughput_ms: throughput,
        iterations,
    })
}

/// Measure Laravel bootstrap time by running `php artisan --version`.
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

/// Measure PHP computation throughput with a synthetic workload.
fn measure_throughput(frankenphp_bin: &str, iterations: u32) -> f64 {
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

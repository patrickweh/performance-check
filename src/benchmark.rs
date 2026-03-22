use crate::supervisor::OctanePorts;
use crate::types::SystemContext;
use colored::Colorize;
use std::net::TcpStream;
use std::process::Command;
use std::sync::{Arc, Barrier};
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
pub fn run_all(frankenphp_bin: &str, app_path: &str, ctx: &SystemContext, octane_ports: &OctanePorts) {
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

    // HTTP load test
    println!("    {}", "HTTP Load Test".bold());
    println!(
        "    {}",
        "Detecting FrankenPHP server...".dimmed()
    );

    match detect_http_port(octane_ports) {
        Some((host, port, is_https)) => {
            let scheme = if is_https { "https" } else { "http" };
            println!(
                "    {}",
                format!("Found server at {scheme}://{host}:{port}").dimmed()
            );
            println!(
                "    {}",
                "Running load test (100 requests, 10 concurrent)...".dimmed()
            );
            println!();

            match run_http_load_test(&host, port, is_https, 100, 10) {
                Some(result) => {
                    for m in &result.metrics {
                        if m.value_ms > 0.0 {
                            println!("      {:<28} {:>8.1}ms", m.label, m.value_ms);
                        } else {
                            println!("      {}", m.label);
                        }
                    }
                }
                None => {
                    println!(
                        "      {}",
                        "Could not complete HTTP load test".yellow()
                    );
                }
            }
        }
        None => {
            println!(
                "      {}",
                format!(
                    "No running FrankenPHP server detected (tried ports {}443, 8000, 8443, 80)",
                    octane_ports.http_port.map(|p| format!("{p}, ")).unwrap_or_default()
                ).yellow()
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
    let select =
        measure_mysql_server_time("SELECT BENCHMARK(100000, MD5('benchmark'))", iterations)?;

    let compute = measure_mysql_server_time("DO BENCHMARK(100000, CRC32('benchmark'))", iterations);

    let mut metrics = vec![Metric {
        label: "SELECT throughput".to_string(),
        value_ms: select,
    }];

    if let Some(w) = compute {
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

/// Measure query execution time server-side using a single mysql process.
/// Runs all iterations in one SQL batch, avoiding fork/exec overhead per iteration.
/// Returns the average time in milliseconds as measured by the MySQL server.
fn measure_mysql_server_time(query: &str, iterations: u32) -> Option<f64> {
    // Build a SQL batch that runs the query N times and reports each timing.
    // Uses MySQL's microsecond timer for accurate measurement.
    let mut sql = String::new();
    for i in 0..iterations {
        sql.push_str(&format!(
            "SET @t{i} = UNIX_TIMESTAMP(NOW(6));\n\
             {query};\n\
             SET @d{i} = (UNIX_TIMESTAMP(NOW(6)) - @t{i}) * 1000;\n"
        ));
    }

    // Collect all timings in one SELECT
    let selects: Vec<String> = (0..iterations).map(|i| format!("@d{i}")).collect();
    sql.push_str(&format!("SELECT {};\n", selects.join(", ")));

    let output = Command::new("mysql")
        .args([
            "--defaults-file=/etc/mysql/debian.cnf",
            "-N",
            "-B",
            "-e",
            &sql,
        ])
        .output()
        .or_else(|_| {
            Command::new("mysql")
                .args(["-u", "root", "-N", "-B", "-e", &sql])
                .output()
        })
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // The last line of output contains tab-separated timing values in ms
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout.lines().last()?;
    let times: Vec<f64> = last_line
        .split('\t')
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .collect();

    if times.is_empty() {
        return None;
    }

    let avg = times.iter().sum::<f64>() / times.len() as f64;
    Some(avg)
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

/// Detect the port FrankenPHP is listening on.
/// Tries supervisor-detected port first, then admin API, then common ports.
fn detect_http_port(octane_ports: &OctanePorts) -> Option<(String, u16, bool)> {
    let host = octane_ports
        .host
        .as_deref()
        .unwrap_or("127.0.0.1");

    // If supervisor config specifies the HTTP port, try it first
    if let Some(port) = octane_ports.http_port {
        let is_https = port == 443 || port == 8443;
        if TcpStream::connect_timeout(
            &format!("{host}:{port}").parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
        {
            return Some((host.to_string(), port, is_https));
        }
    }

    // Check admin API for server listen address
    if let Some(port) = detect_port_from_admin_api(octane_ports) {
        return Some((host.to_string(), port, port == 443 || port == 8443));
    }

    // Try common ports as fallback
    let candidates = [(443, true), (8000, false), (8443, true), (80, false)];

    for (port, is_https) in candidates {
        if TcpStream::connect_timeout(
            &format!("{host}:{port}").parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
        {
            return Some((host.to_string(), port, is_https));
        }
    }

    None
}

/// Try to detect the HTTP port from the Caddy admin API.
fn detect_port_from_admin_api(octane_ports: &OctanePorts) -> Option<u16> {
    // Build admin port list: supervisor-detected first, then defaults
    let mut admin_ports = Vec::with_capacity(3);
    if let Some(p) = octane_ports.admin_port {
        admin_ports.push(p);
    }
    for default in [2019u16, 2020] {
        if !admin_ports.contains(&default) {
            admin_ports.push(default);
        }
    }

    for admin_port in admin_ports {
        let output = Command::new("curl")
            .args([
                "-s",
                "--connect-timeout",
                "1",
                "--max-time",
                "2",
                &format!("http://localhost:{admin_port}/config/apps/http/servers"),
            ])
            .output()
            .ok()?;

        if !output.status.success() {
            continue;
        }

        let body = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Look for listen addresses in any server
        for (_name, server) in json.as_object()? {
            if let Some(listen) = server.get("listen").and_then(|l| l.as_array()) {
                for addr in listen {
                    if let Some(addr_str) = addr.as_str() {
                        // Format is typically ":443" or ":8000"
                        if let Some(port_str) = addr_str.strip_prefix(':') {
                            if let Ok(port) = port_str.parse::<u16>() {
                                return Some(port);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Run an HTTP load test against the FrankenPHP server.
///
/// Uses curl for HTTPS support (TLS is complex with raw sockets).
/// Sends `total_requests` across `concurrency` threads.
fn run_http_load_test(
    host: &str,
    port: u16,
    is_https: bool,
    total_requests: u32,
    concurrency: u32,
) -> Option<BenchmarkResult> {
    let scheme = if is_https { "https" } else { "http" };
    let url = format!("{scheme}://{host}:{port}/up");

    // Warm up: single request to ensure we get a valid response
    let warmup = Command::new("curl")
        .args(["-sk", "--connect-timeout", "3", "--max-time", "5", "-o", "/dev/null", "-w", "%{http_code}", &url])
        .output()
        .ok()?;

    let status = String::from_utf8_lossy(&warmup.stdout).trim().to_string();
    if !warmup.status.success() || (!status.starts_with('2') && !status.starts_with('3')) {
        // Try root path instead
        let url_root = format!("{scheme}://{host}:{port}/");
        let warmup2 = Command::new("curl")
            .args(["-sk", "--connect-timeout", "3", "--max-time", "5", "-o", "/dev/null", "-w", "%{http_code}", &url_root])
            .output()
            .ok()?;

        let status2 = String::from_utf8_lossy(&warmup2.stdout).trim().to_string();
        if !warmup2.status.success() || (!status2.starts_with('2') && !status2.starts_with('3')) {
            return None;
        }

        return run_http_load_test_inner(&url_root, total_requests, concurrency);
    }

    run_http_load_test_inner(&url, total_requests, concurrency)
}

fn run_http_load_test_inner(
    url: &str,
    total_requests: u32,
    concurrency: u32,
) -> Option<BenchmarkResult> {
    let requests_per_thread = total_requests / concurrency;
    let url = Arc::new(url.to_string());
    let barrier = Arc::new(Barrier::new(concurrency as usize));

    let start = Instant::now();

    let handles: Vec<_> = (0..concurrency)
        .map(|_| {
            let url = Arc::clone(&url);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut latencies = Vec::with_capacity(requests_per_thread as usize);
                let mut errors = 0u32;

                barrier.wait(); // Synchronize start

                for _ in 0..requests_per_thread {
                    let req_start = Instant::now();
                    let result = Command::new("curl")
                        .args([
                            "-sk",
                            "--connect-timeout",
                            "3",
                            "--max-time",
                            "10",
                            "-o",
                            "/dev/null",
                            "-w",
                            "%{http_code}",
                            &url,
                        ])
                        .output();

                    let elapsed = req_start.elapsed();

                    match result {
                        Ok(out) if out.status.success() => {
                            latencies.push(elapsed);
                        }
                        _ => {
                            errors += 1;
                        }
                    }
                }

                (latencies, errors)
            })
        })
        .collect();

    let mut all_latencies = Vec::new();
    let mut total_errors = 0u32;

    for h in handles {
        if let Ok((latencies, errors)) = h.join() {
            all_latencies.extend(latencies);
            total_errors += errors;
        }
    }

    let total_time = start.elapsed();

    if all_latencies.is_empty() {
        return None;
    }

    all_latencies.sort();

    let successful = all_latencies.len() as f64;
    let rps = successful / total_time.as_secs_f64();
    let avg_latency = avg_ms(&all_latencies);
    let p50 = all_latencies[all_latencies.len() / 2].as_secs_f64() * 1000.0;
    let p99_idx = ((all_latencies.len() as f64) * 0.99) as usize;
    let p99 = all_latencies[p99_idx.min(all_latencies.len() - 1)].as_secs_f64() * 1000.0;

    let mut metrics = vec![
        Metric {
            label: format!("Requests/sec: {rps:.1} ({concurrency}c)"),
            value_ms: 0.0,
        },
        Metric {
            label: "Avg latency".to_string(),
            value_ms: avg_latency,
        },
        Metric {
            label: "P50 latency".to_string(),
            value_ms: p50,
        },
        Metric {
            label: "P99 latency".to_string(),
            value_ms: p99,
        },
    ];

    if total_errors > 0 {
        metrics.push(Metric {
            label: format!("Errors: {total_errors}/{total_requests}"),
            value_ms: 0.0,
        });
    }

    Some(BenchmarkResult {
        kind: "HTTP Load Test",
        metrics,
    })
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

# frankenphp-check

CLI tool that audits a server for FrankenPHP + Laravel/Flux performance optimizations and outputs concrete, context-aware recommendations.

## Install

Download the latest binary from [Releases](../../releases):

```bash
# amd64
wget https://github.com/patrickweh/performance-check/releases/latest/download/frankenphp-check-linux-amd64
chmod +x frankenphp-check-linux-amd64
sudo mv frankenphp-check-linux-amd64 /usr/local/bin/frankenphp-check

# arm64
wget https://github.com/patrickweh/performance-check/releases/latest/download/frankenphp-check-linux-arm64
chmod +x frankenphp-check-linux-arm64
sudo mv frankenphp-check-linux-arm64 /usr/local/bin/frankenphp-check
```

## Usage

```bash
# Auto-discover Laravel apps from nginx/Caddy configs
sudo frankenphp-check

# Specify app path explicitly
sudo frankenphp-check /home/forge/app.com

# Apply fixes interactively (with before/after benchmarks)
sudo frankenphp-check --fix

# Run benchmarks only (PHP + MySQL)
sudo frankenphp-check --bench

# JSON output for automation
frankenphp-check --json
```

`sudo` is needed to read process environments, MySQL configs, and apply fixes.

```
frankenphp-check [OPTIONS] [APP_PATH]

Arguments:
  [APP_PATH]  Path to the Laravel app (auto-discovered if omitted)

Options:
      --frankenphp <PATH>  Path to FrankenPHP binary [default: /usr/bin/frankenphp]
      --php-ini <PATH>     Path to php-zts php.ini [default: /etc/php-zts/php.ini]
      --json               Output as JSON
      --no-color           Disable ANSI colors
      --fix                Interactively apply suggested fixes
      --bench              Run all benchmarks (PHP + MySQL)
```

## What it checks

| Category | Checks |
|---|---|
| **System** | CPU cores, RAM, swap usage, co-located services (MySQL, Redis), PHP RAM budget calculation |
| **libc** | musl (FAIL) vs glibc — musl is significantly slower for PHP-ZTS |
| **FrankenPHP** | Binary exists, version |
| **PHP-ZTS** | Extensions (bcmath, pdo, pdo_mysql, redis, gd, intl, zip, opcache) |
| **OPcache** | enable, validate_timestamps, memory_consumption, max_accelerated_files, interned_strings_buffer, jit_buffer_size, preload |
| **Realpath Cache** | realpath_cache_size (≥4096K), realpath_cache_ttl (≥600) |
| **PHP memory_limit** | Value check + worker memory risk calculation vs RAM budget |
| **Go Runtime** | GODEBUG=cgocheck=0, GOMEMLIMIT |
| **Laravel .env** | APP_ENV, APP_DEBUG, OCTANE_HTTPS, CACHE_STORE, QUEUE_CONNECTION, SESSION_DRIVER, LOG_CHANNEL (warns on stack/file-based) |
| **Bootstrap Cache** | config.php, routes-v7.php, services.php (+ packages.php for Laravel <11) |
| **Composer** | Optimized autoloader classmap |
| **MySQL/MariaDB** | Auto-detects .cnf in /etc/mysql/conf.d/. Checks: innodb_buffer_pool_size, innodb_log_file_size, query_cache_type (only <8.0), slow_query_log, max_connections, innodb_flush_log_at_trx_commit, tmp_table_size |
| **Redis** | maxmemory, maxmemory-policy |
| **File Descriptors** | ulimit -n (≥65536) |

## Benchmarks (`--bench`)

Runs performance benchmarks without applying any fixes:

| Benchmark | What it measures |
|---|---|
| **Laravel Bootstrap** | Cold start time via `php artisan --version` (5 iterations) |
| **PHP Compute** | Synthetic workload: sqrt loop + md5 hashing |
| **MySQL SELECT** | `BENCHMARK(100000, MD5(...))` throughput |
| **MySQL Compute** | `BENCHMARK(100000, CRC32(...))` throughput |
| **InnoDB Buffer Pool** | Hit rate percentage |

## Interactive Fixes (`--fix`)

When fixes are available, you can choose how to apply them:

- **Try with benchmark** — Backs up the file, runs a before/after benchmark specific to the fix type (PHP or MySQL), shows comparison, then asks to keep or revert
- **Apply without benchmark** — Applies the fix directly
- **Skip** — Skips the fix

On revert, the original file is restored byte-for-byte. If the file didn't exist before, it is removed.

The benchmark option is only offered for fixes where it's meaningful (php.ini, MySQL config). For .env changes there's no benchmark since the impact isn't measurable via CLI.

App code is never modified.

## Auto-Discovery

When no `APP_PATH` is given, the tool searches for Laravel apps in:

1. `/etc/nginx/sites-enabled/*` — parses `root` directives, strips `/public`
2. `/etc/caddy/Caddyfile` and `/etc/frankenphp/Caddyfile` — parses `root` directives
3. `/home/forge/*/artisan` — Forge convention fallback

Multiple discovered apps are checked sequentially.

## Output

Results are grouped by section with colored status badges:

```
  FrankenPHP + Laravel Performance Check

  System

     PASS  CPU Cores  4
     PASS  Swap Usage  No swap in use
     INFO  PHP RAM Budget  3072MB (Total 4096 - OS 512 - ...)

  PHP Configuration

     PASS  opcache.enable
     WARN  realpath_cache_ttl
            120 — recommend ≥600
     FAIL  APP_DEBUG
            'true' — MUST be false in production

  Results

    ████████ Pass: 28  ░░░░░░░░ Warn: 3  ░░░░░░░░ Fail: 1

    Score: 85%
```

Exit code is `1` if any check fails, `0` otherwise.

## Build from source

```bash
cargo build --release
```

Binary is at `target/release/frankenphp-check`.

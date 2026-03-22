mod checks;
mod fixes;
mod output;
mod types;

use clap::Parser;

#[derive(Parser)]
#[command(name = "frankenphp-check", about = "FrankenPHP + Laravel/Flux Performance Checker")]
struct Cli {
    /// Path to the Laravel app (e.g. /home/forge/app.com)
    app_path: String,

    /// Path to the FrankenPHP binary
    #[arg(long, default_value = "/usr/bin/frankenphp")]
    frankenphp: String,

    /// Path to the php-zts php.ini
    #[arg(long, default_value = "/etc/php-zts/php.ini")]
    php_ini: String,

    /// Output as JSON instead of text
    #[arg(long)]
    json: bool,

    /// Disable ANSI color output
    #[arg(long)]
    no_color: bool,

    /// Interactively apply suggested fixes
    #[arg(long)]
    fix: bool,
}

fn main() {
    let cli = Cli::parse();

    let mut all_results = Vec::new();

    // 1. System context (foundation for all other checks)
    let (ctx, system_results) = checks::system::gather(&cli.app_path);
    all_results.extend(system_results);

    // 2. libc check
    all_results.extend(checks::libc::check());

    // 3. FrankenPHP binary
    all_results.extend(checks::frankenphp::check(&cli.frankenphp));

    // 4. PHP-ZTS, OPcache, Realpath
    all_results.extend(checks::php::check(&cli.frankenphp, &cli.php_ini, &ctx));

    // 5. Go runtime
    all_results.extend(checks::go_runtime::check(&ctx));

    // 6. Laravel .env, bootstrap cache, composer
    all_results.extend(checks::laravel::check(&cli.app_path, &ctx));

    // 7. MySQL
    all_results.extend(checks::mysql::check(&ctx));

    // 8. Redis
    all_results.extend(checks::redis::check(&ctx));

    // Output
    output::print_results(&all_results, !cli.no_color, cli.json);

    // Interactive fixes
    if cli.fix && !cli.json {
        fixes::propose_interactive_fixes(&all_results);
    }

    // Exit code based on results
    let has_fail = all_results.iter().any(|r| r.status == types::Status::Fail);
    if has_fail {
        std::process::exit(1);
    }
}

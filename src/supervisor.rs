use std::fs;
use std::path::Path;

/// Ports extracted from a Forge/Supervisor octane:start command.
#[derive(Debug, Default, Clone)]
pub struct OctanePorts {
    /// HTTP port (--port flag), e.g. 8000
    pub http_port: Option<u16>,
    /// Caddy admin API port (--admin-port flag), e.g. 2019
    pub admin_port: Option<u16>,
    /// Host (--host flag), e.g. 127.0.0.1
    pub host: Option<String>,
}

/// Detect Octane ports from Supervisor configs and running processes.
///
/// Search order:
/// 1. Running processes (ps) for artisan octane:start
/// 2. Supervisor config files in /etc/supervisor/conf.d/
pub fn detect_octane_ports(app_path: &str) -> OctanePorts {
    // Try running processes first (most accurate — reflects reality)
    if let Some(ports) = detect_from_process(app_path) {
        return ports;
    }

    // Fall back to supervisor config files
    if let Some(ports) = detect_from_supervisor_configs(app_path) {
        return ports;
    }

    OctanePorts::default()
}

/// Parse octane:start flags from running processes.
fn detect_from_process(app_path: &str) -> Option<OctanePorts> {
    let output = std::process::Command::new("ps")
        .args(["--no-headers", "-eo", "args"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if !line.contains("octane:start") {
            continue;
        }
        // Match against app_path if present in the command, or accept any octane:start
        if !app_path.is_empty() && !line.contains(app_path) && !line.contains("artisan") {
            continue;
        }
        if line.contains("frankenphp")
            || line.contains("--server=frankenphp")
            || line.contains("octane:start")
        {
            return Some(parse_octane_flags(line));
        }
    }

    None
}

/// Scan /etc/supervisor/conf.d/ for octane:start commands.
fn detect_from_supervisor_configs(app_path: &str) -> Option<OctanePorts> {
    let conf_dir = Path::new("/etc/supervisor/conf.d");
    if !conf_dir.is_dir() {
        return None;
    }

    let entries = fs::read_dir(conf_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Look for command= lines containing octane:start
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with("command=") {
                continue;
            }
            let cmd = &trimmed["command=".len()..];
            if !cmd.contains("octane:start") {
                continue;
            }
            // If app_path is specified, try to match it
            if !app_path.is_empty() && !cmd.contains(app_path) {
                // Still parse it as a fallback candidate
            }
            if cmd.contains(app_path) || app_path.is_empty() {
                return Some(parse_octane_flags(cmd));
            }
        }
    }

    // Second pass: if no exact app_path match, take first octane:start with frankenphp
    let entries = fs::read_dir(conf_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with("command=") {
                continue;
            }
            let cmd = &trimmed["command=".len()..];
            if cmd.contains("octane:start") && cmd.contains("frankenphp") {
                return Some(parse_octane_flags(cmd));
            }
        }
    }

    None
}

/// Extract --port, --admin-port, and --host flags from an octane:start command line.
fn parse_octane_flags(cmd: &str) -> OctanePorts {
    let mut ports = OctanePorts::default();
    let parts: Vec<&str> = cmd.split_whitespace().collect();

    for (i, part) in parts.iter().enumerate() {
        // Handle --port=8000 and --port 8000 forms
        if let Some(val) = part.strip_prefix("--port=") {
            ports.http_port = val.parse().ok();
        } else if *part == "--port" {
            if let Some(val) = parts.get(i + 1) {
                ports.http_port = val.parse().ok();
            }
        }

        if let Some(val) = part.strip_prefix("--admin-port=") {
            ports.admin_port = val.parse().ok();
        } else if *part == "--admin-port" {
            if let Some(val) = parts.get(i + 1) {
                ports.admin_port = val.parse().ok();
            }
        }

        if let Some(val) = part.strip_prefix("--host=") {
            ports.host = Some(val.to_string());
        } else if *part == "--host" {
            if let Some(val) = parts.get(i + 1) {
                ports.host = Some(val.to_string());
            }
        }
    }

    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flags_equals_form() {
        let cmd = "php /home/forge/app.com/artisan octane:start --server=frankenphp --host=127.0.0.1 --port=8000 --admin-port=2019";
        let ports = parse_octane_flags(cmd);
        assert_eq!(ports.http_port, Some(8000));
        assert_eq!(ports.admin_port, Some(2019));
        assert_eq!(ports.host.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn parse_flags_space_form() {
        let cmd = "php artisan octane:start --server frankenphp --host 0.0.0.0 --port 9000 --admin-port 2020";
        let ports = parse_octane_flags(cmd);
        assert_eq!(ports.http_port, Some(9000));
        assert_eq!(ports.admin_port, Some(2020));
        assert_eq!(ports.host.as_deref(), Some("0.0.0.0"));
    }

    #[test]
    fn parse_flags_minimal() {
        let cmd = "php artisan octane:start --server=frankenphp";
        let ports = parse_octane_flags(cmd);
        assert_eq!(ports.http_port, None);
        assert_eq!(ports.admin_port, None);
        assert_eq!(ports.host, None);
    }

    #[test]
    fn parse_flags_only_port() {
        let cmd = "/usr/bin/php /home/forge/example.com/artisan octane:start --server=frankenphp --port=443";
        let ports = parse_octane_flags(cmd);
        assert_eq!(ports.http_port, Some(443));
        assert_eq!(ports.admin_port, None);
    }

    #[test]
    fn parse_flags_mixed_form() {
        let cmd = "php artisan octane:start --server=frankenphp --port 8080 --admin-port=3000 --host=127.0.0.1";
        let ports = parse_octane_flags(cmd);
        assert_eq!(ports.http_port, Some(8080));
        assert_eq!(ports.admin_port, Some(3000));
        assert_eq!(ports.host.as_deref(), Some("127.0.0.1"));
    }
}

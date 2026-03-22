use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Auto-discover Laravel app paths from server configs.
/// Searches (in order):
/// 1. Nginx sites-enabled configs (root directive → strip /public)
/// 2. Caddyfile (root directive → strip /public)
/// 3. Forge convention: /home/forge/*/artisan
pub fn find_laravel_apps() -> Vec<String> {
    let mut apps = BTreeSet::new();

    // 1. Nginx
    apps.extend(from_nginx());

    // 2. Caddyfile
    apps.extend(from_caddyfile());

    // 3. Forge fallback: scan /home/forge/*/artisan
    if apps.is_empty() {
        apps.extend(from_forge_convention());
    }

    apps.into_iter().collect()
}

/// Parse nginx configs in /etc/nginx/sites-enabled/
/// Looking for: root /home/forge/app.com/public;
fn from_nginx() -> Vec<String> {
    let dir = "/etc/nginx/sites-enabled";
    let mut apps = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return apps,
    };

    for entry in entries.flatten() {
        let content = match fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for path in extract_root_directives(&content) {
            if is_laravel_app(&path) {
                apps.push(path);
            }
        }
    }

    apps
}

/// Parse Caddyfile (typically /etc/caddy/Caddyfile)
/// Looking for: root * /home/forge/app.com/public
fn from_caddyfile() -> Vec<String> {
    let paths = ["/etc/caddy/Caddyfile", "/etc/frankenphp/Caddyfile"];
    let mut apps = Vec::new();

    for caddy_path in &paths {
        let content = match fs::read_to_string(caddy_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines() {
            let trimmed = line.trim();
            // Match: root * /path/to/public
            // or:    root /path/to/public
            if trimmed.starts_with("root") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if let Some(last) = parts.last() {
                    let app_path = strip_public(last);
                    if is_laravel_app(&app_path) {
                        apps.push(app_path);
                    }
                }
            }
        }
    }

    apps
}

/// Fallback: scan /home/forge/*/artisan
fn from_forge_convention() -> Vec<String> {
    let forge_home = "/home/forge";
    let mut apps = Vec::new();

    let entries = match fs::read_dir(forge_home) {
        Ok(e) => e,
        Err(_) => return apps,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("artisan").exists() {
            if let Some(s) = path.to_str() {
                apps.push(s.to_string());
            }
        }
    }

    apps
}

/// Extract root paths from nginx config.
/// Matches lines like: root /home/forge/app.com/public;
fn extract_root_directives(content: &str) -> Vec<String> {
    let mut roots = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("root") && trimmed.contains('/') {
            // root /path/to/public;
            let after_root = trimmed.trim_start_matches("root").trim();
            let path = after_root.trim_end_matches(';').trim();
            let app_path = strip_public(path);
            if !app_path.is_empty() {
                roots.push(app_path);
            }
        }
    }

    roots
}

/// Strip trailing /public from a web root to get the app root
fn strip_public(path: &str) -> String {
    let path = path.trim_end_matches('/');
    if path.ends_with("/public") {
        path[..path.len() - 7].to_string()
    } else {
        path.to_string()
    }
}

/// Quick check: is this actually a Laravel app?
fn is_laravel_app(path: &str) -> bool {
    Path::new(path).join("artisan").exists()
}

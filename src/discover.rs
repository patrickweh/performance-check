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
    if let Some(stripped) = path.strip_suffix("/public") {
        stripped.to_string()
    } else {
        path.to_string()
    }
}

/// Quick check: is this actually a Laravel app?
fn is_laravel_app(path: &str) -> bool {
    Path::new(path).join("artisan").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_public_removes_suffix() {
        assert_eq!(
            strip_public("/home/forge/app.com/public"),
            "/home/forge/app.com"
        );
    }

    #[test]
    fn strip_public_removes_trailing_slash() {
        assert_eq!(
            strip_public("/home/forge/app.com/public/"),
            "/home/forge/app.com"
        );
    }

    #[test]
    fn strip_public_no_public_suffix() {
        assert_eq!(strip_public("/home/forge/app.com"), "/home/forge/app.com");
    }

    #[test]
    fn strip_public_only_public() {
        assert_eq!(strip_public("/public"), "");
    }

    #[test]
    fn extract_nginx_root_directive() {
        let config = r#"
server {
    listen 80;
    server_name app.com;
    root /home/forge/app.com/public;

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }
}
"#;
        let roots = extract_root_directives(config);
        assert_eq!(roots, vec!["/home/forge/app.com"]);
    }

    #[test]
    fn extract_multiple_nginx_roots() {
        let config = r#"
server {
    root /home/forge/site1.com/public;
}
server {
    root /home/forge/site2.com/public;
}
"#;
        let roots = extract_root_directives(config);
        assert_eq!(
            roots,
            vec!["/home/forge/site1.com", "/home/forge/site2.com"]
        );
    }

    #[test]
    fn extract_nginx_root_without_public() {
        let config = "    root /var/www/html;\n";
        let roots = extract_root_directives(config);
        assert_eq!(roots, vec!["/var/www/html"]);
    }

    #[test]
    fn extract_nginx_ignores_commented_root() {
        let config = "    # root /old/path/public;\n    root /new/path/public;\n";
        let roots = extract_root_directives(config);
        assert_eq!(roots, vec!["/new/path"]);
    }

    #[test]
    fn is_laravel_app_with_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_laravel_app(dir.path().to_str().unwrap()));

        std::fs::write(dir.path().join("artisan"), "#!/usr/bin/env php\n").unwrap();
        assert!(is_laravel_app(dir.path().to_str().unwrap()));
    }

    // --- Additional strip_public edge cases ---

    #[test]
    fn strip_public_multiple_trailing_slashes() {
        // trim_end_matches('/') strips all trailing slashes, then strip_suffix("/public")
        assert_eq!(
            strip_public("/home/forge/app.com/public///"),
            "/home/forge/app.com"
        );
    }

    #[test]
    fn strip_public_nested_public() {
        // /public/public should only strip the last one
        assert_eq!(strip_public("/public/public"), "/public");
    }

    #[test]
    fn strip_public_empty_string() {
        assert_eq!(strip_public(""), "");
    }

    #[test]
    fn strip_public_just_slash() {
        assert_eq!(strip_public("/"), "");
    }

    // --- Nginx parsing edge cases ---

    #[test]
    fn extract_nginx_root_with_tabs() {
        let config = "\t\troot\t/home/forge/app.com/public;\n";
        let roots = extract_root_directives(config);
        assert_eq!(roots, vec!["/home/forge/app.com"]);
    }

    #[test]
    fn extract_nginx_root_no_semicolon() {
        // Caddyfile-style without semicolon
        let config = "    root /home/forge/app.com/public\n";
        let roots = extract_root_directives(config);
        assert_eq!(roots, vec!["/home/forge/app.com"]);
    }

    #[test]
    fn extract_nginx_empty_config() {
        assert_eq!(extract_root_directives("").len(), 0);
    }

    #[test]
    fn extract_nginx_root_with_extra_spaces() {
        let config = "    root    /home/forge/app.com/public  ;  \n";
        let roots = extract_root_directives(config);
        assert!(!roots.is_empty());
    }

    #[test]
    fn extract_nginx_root_inside_location_block() {
        let config = r#"
server {
    listen 80;
    root /home/forge/main/public;

    location /admin {
        root /home/forge/admin/public;
    }
}
"#;
        let roots = extract_root_directives(config);
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&"/home/forge/main".to_string()));
        assert!(roots.contains(&"/home/forge/admin".to_string()));
    }

    #[test]
    fn extract_nginx_no_root_directive() {
        let config = r#"
server {
    listen 80;
    server_name example.com;
    proxy_pass http://backend;
}
"#;
        let roots = extract_root_directives(config);
        assert!(roots.is_empty());
    }

    #[test]
    fn extract_nginx_root_with_variable_ignored() {
        // Lines with $variables don't contain '/' so they're filtered out
        let config = "    root $document_root;\n";
        let roots = extract_root_directives(config);
        assert!(
            roots.is_empty(),
            "$variable paths are ignored (no / in path)"
        );
    }

    // --- Full nginx config scenario ---

    #[test]
    fn extract_realistic_nginx_config() {
        let config = r#"
server {
    listen 80;
    listen [::]:80;
    server_name example.com www.example.com;

    # Redirect to HTTPS
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name example.com;

    ssl_certificate /etc/letsencrypt/live/example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/example.com/privkey.pem;

    root /home/forge/example.com/public;

    add_header X-Frame-Options "SAMEORIGIN";
    add_header X-Content-Type-Options "nosniff";

    index index.php;

    charset utf-8;

    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }

    location = /favicon.ico { access_log off; log_not_found off; }
    location = /robots.txt  { access_log off; log_not_found off; }

    error_page 404 /index.php;

    location ~ \.php$ {
        fastcgi_pass unix:/var/run/php/php8.2-fpm.sock;
        fastcgi_param SCRIPT_FILENAME $realpath_root$fastcgi_script_name;
        include fastcgi_params;
    }

    location ~ /\.(?!well-known).* {
        deny all;
    }
}
"#;
        let roots = extract_root_directives(config);
        assert_eq!(roots, vec!["/home/forge/example.com"]);
    }
}

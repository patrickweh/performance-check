use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Info,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Ok => write!(f, "OK  "),
            Status::Warn => write!(f, "WARN"),
            Status::Fail => write!(f, "FAIL"),
            Status::Info => write!(f, "INFO"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub status: Status,
    pub label: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    pub description: String,
    pub file: String,
    pub content: String,
}

impl CheckResult {
    pub fn ok(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: Status::Ok,
            label: label.into(),
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn warn(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: Status::Warn,
            label: label.into(),
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn fail(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: Status::Fail,
            label: label.into(),
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn info(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: Status::Info,
            label: label.into(),
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn with_fix(
        mut self,
        description: impl Into<String>,
        file: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.fix = Some(Fix {
            description: description.into(),
            file: file.into(),
            content: content.into(),
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_result_ok() {
        let r = CheckResult::ok("test", "detail");
        assert_eq!(r.status, Status::Ok);
        assert_eq!(r.label, "test");
        assert_eq!(r.detail, "detail");
        assert!(r.fix.is_none());
    }

    #[test]
    fn check_result_with_fix() {
        let r = CheckResult::warn("opcache.enable", "Off").with_fix(
            "Enable opcache",
            "/etc/php.ini",
            "opcache.enable=1",
        );
        assert_eq!(r.status, Status::Warn);
        let fix = r.fix.unwrap();
        assert_eq!(fix.description, "Enable opcache");
        assert_eq!(fix.file, "/etc/php.ini");
        assert_eq!(fix.content, "opcache.enable=1");
    }

    #[test]
    fn check_result_fail() {
        let r = CheckResult::fail("APP_DEBUG", "true");
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn check_result_info() {
        let r = CheckResult::info("CPU Cores", "4");
        assert_eq!(r.status, Status::Info);
    }

    #[test]
    fn status_display() {
        assert_eq!(format!("{}", Status::Ok), "OK  ");
        assert_eq!(format!("{}", Status::Warn), "WARN");
        assert_eq!(format!("{}", Status::Fail), "FAIL");
        assert_eq!(format!("{}", Status::Info), "INFO");
    }

    #[test]
    fn check_result_serialization() {
        let r = CheckResult::ok("test", "value");
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"label\":\"test\""));
        // fix should be omitted when None
        assert!(!json.contains("\"fix\""));
    }

    #[test]
    fn check_result_with_fix_serialization() {
        let r = CheckResult::warn("key", "detail").with_fix("desc", "file", "content");
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"fix\""));
        assert!(json.contains("\"description\":\"desc\""));
    }
}

/// System context gathered once and passed to all checks.
#[derive(Debug, Clone, Serialize)]
pub struct SystemContext {
    pub cpu_cores: usize,
    pub total_ram_mb: u64,
    pub available_ram_mb: u64,
    pub swap_used_mb: u64,
    pub mysql_running: bool,
    pub mysql_pid: Option<u32>,
    pub mysql_ram_mb: u64,
    pub redis_running: bool,
    pub redis_pid: Option<u32>,
    pub redis_ram_mb: u64,
    pub php_ram_budget_mb: u64,
    pub laravel_version: Option<String>,
    pub laravel_major: Option<u32>,
}

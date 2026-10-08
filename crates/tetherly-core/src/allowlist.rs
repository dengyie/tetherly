// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CoreError;
use serde::Deserialize;
use std::path::{Component, Path};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenRule {
    pub app_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct OpenAllowlist {
    rules: Vec<OpenRule>,
}

impl OpenAllowlist {
    pub fn from_rules(rules: Vec<OpenRule>) -> Result<Self, CoreError> {
        for r in &rules {
            if r.url.contains("://") && !is_local_scheme(&r.url) {
                return Err(CoreError::Json("remote url in allowlist".into()));
            }
            if r.url.contains("..") {
                return Err(CoreError::UnsafeFileName);
            }
        }
        Ok(Self { rules })
    }

    pub fn url_for(&self, app_id: &str) -> Option<&str> {
        self.rules
            .iter()
            .find(|r| glob_match(&r.app_id, app_id))
            .map(|r| r.url.as_str())
    }
}

fn is_local_scheme(url: &str) -> bool {
    let scheme = url.split(':').next().unwrap_or("");
    matches!(
        scheme,
        "weixin" | "wechat" | "alipay" | "taobao" | "tetherly"
    ) && !url.starts_with("http://")
        && !url.starts_with("https://")
}

fn glob_match(pattern: &str, value: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        value.starts_with(prefix)
    } else {
        pattern == value
    }
}

pub fn sanitize_file_name(name: &str) -> Result<String, CoreError> {
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(CoreError::UnsafeFileName)?;
    if base.is_empty() || base == "." || base == ".." || base.contains('\0') {
        return Err(CoreError::UnsafeFileName);
    }
    for c in Path::new(base).components() {
        if !matches!(c, Component::Normal(_)) {
            return Err(CoreError::UnsafeFileName);
        }
    }
    Ok(base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_url_ok_remote_rejected_at_lookup_time() {
        let list = OpenAllowlist::from_rules(vec![OpenRule {
            app_id: "com.tencent.xin".into(),
            url: "weixin://".into(),
        }])
        .unwrap();
        assert_eq!(list.url_for("com.tencent.xin"), Some("weixin://"));
        assert_eq!(list.url_for("other"), None);
        assert!(OpenAllowlist::from_rules(vec![OpenRule {
            app_id: "x".into(),
            url: "https://evil.example".into(),
        }])
        .is_err());
    }

    #[test]
    fn basename_only() {
        assert_eq!(sanitize_file_name("a.txt").unwrap(), "a.txt");
        assert_eq!(sanitize_file_name("/tmp/../etc/passwd").unwrap(), "passwd");
        assert!(sanitize_file_name("..").is_err());
        assert!(sanitize_file_name("").is_err());
    }
}

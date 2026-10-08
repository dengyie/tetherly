// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Conservative write plan. Spec §9.2: empty → write all; prefix → complete
//! remainder; already complete → do not write; read-only → refuse. Anything
//! else is refused (no append). Callers must invoke this only on a user click.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertionDecision {
    Write,
    DoNotTouch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertionPlan {
    pub decision: InsertionDecision,
    pub value: Option<String>,
    pub typed: Option<String>,
    pub reason: String,
}

impl InsertionPlan {
    pub fn should_write(&self) -> bool {
        self.decision == InsertionDecision::Write
    }

    fn do_not_touch(reason: impl Into<String>) -> Self {
        Self {
            decision: InsertionDecision::DoNotTouch,
            value: None,
            typed: None,
            reason: reason.into(),
        }
    }

    fn write(
        value: impl Into<String>,
        typed: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            decision: InsertionDecision::Write,
            value: Some(value.into()),
            typed: Some(typed.into()),
            reason: reason.into(),
        }
    }
}

pub fn plan(current_value: Option<&str>, code: Option<&str>, read_only: bool) -> InsertionPlan {
    if read_only {
        return InsertionPlan::do_not_touch("只读则拒绝");
    }

    let Some(code) = code.filter(|code| !code.is_empty()) else {
        return InsertionPlan::do_not_touch("没有可用的验证码");
    };

    let Some(current_value) = current_value else {
        return InsertionPlan::write(code, code, "字段是空的，直接写入");
    };

    if current_value.is_empty() || current_value.trim().is_empty() {
        return InsertionPlan::write(code, code, "字段是空的，直接写入");
    }

    let trimmed = current_value.trim();
    if trimmed == code {
        return InsertionPlan::do_not_touch("已完整则不写");
    }

    if let Some(tail) = code.strip_prefix(trimmed) {
        if !tail.is_empty() {
            return InsertionPlan::write(code, tail, "前缀则补差");
        }
    }

    InsertionPlan::do_not_touch("无法安全写入")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_writes_all() {
        let p = plan(Some(""), Some("868740"), false);
        assert!(p.should_write());
        assert_eq!(p.value.as_deref(), Some("868740"));
        assert_eq!(p.typed.as_deref(), Some("868740"));
    }

    #[test]
    fn none_current_is_empty() {
        let p = plan(None, Some("868740"), false);
        assert!(p.should_write());
        assert_eq!(p.value.as_deref(), Some("868740"));
    }

    #[test]
    fn prefix_completes() {
        let p = plan(Some("868"), Some("868740"), false);
        assert!(p.should_write());
        assert_eq!(p.value.as_deref(), Some("868740"));
        assert_eq!(p.typed.as_deref(), Some("740"));
    }

    #[test]
    fn complete_does_not_write() {
        let p = plan(Some("868740"), Some("868740"), false);
        assert!(!p.should_write());
    }

    #[test]
    fn read_only_refuses() {
        let p = plan(Some(""), Some("868740"), true);
        assert!(!p.should_write());
        assert_eq!(p.reason, "只读则拒绝");
    }

    #[test]
    fn unrelated_content_is_refused() {
        let p = plan(Some("hello"), Some("868740"), false);
        assert!(!p.should_write());
    }

    #[test]
    fn missing_code_refuses() {
        assert!(!plan(Some(""), None, false).should_write());
        assert!(!plan(Some(""), Some(""), false).should_write());
    }
}

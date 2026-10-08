// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OTP extractor frozen to sms-pop-rs scoring. Do not "improve" the rules:
//! 4–8 digit runs, keyword windows 48/24, skip 11-digit phones, drop 1900–2099
//! years, prefer miss over a wrong code.

const KEYWORDS: [&str; 11] = [
    "验证码",
    "校验码",
    "动态码",
    "动态密码",
    "安全码",
    "确认码",
    "短信密码",
    "verification code",
    "one-time code",
    "security code",
    "otp",
];

pub const DEFAULT_MIN_SCORE: i32 = 5;
const KEYWORD_BONUS_BEFORE: usize = 48;
const KEYWORD_BONUS_AFTER: usize = 24;

pub trait OtpExtractor: Send + Sync {
    fn extract(&self, text: &str) -> Option<String>;
}

#[derive(Debug, Clone)]
pub struct DefaultOtpExtractor {
    pub min_score: i32,
}

impl Default for DefaultOtpExtractor {
    fn default() -> Self {
        Self {
            min_score: DEFAULT_MIN_SCORE,
        }
    }
}

impl OtpExtractor for DefaultOtpExtractor {
    fn extract(&self, text: &str) -> Option<String> {
        extract_otp_with_min_score(text, self.min_score)
    }
}

pub fn extract_otp(text: &str) -> Option<String> {
    extract_otp_with_min_score(text, DEFAULT_MIN_SCORE)
}

pub fn extract_otp_with_min_score(text: &str, min_score: i32) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }

    let bytes = text.as_bytes();
    let keyword_positions = find_keywords(text);

    let mut best: Option<(i32, String)> = None;
    let mut index = 0usize;

    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }

        let length = index - start;
        if !(4..=8).contains(&length) {
            continue;
        }

        let code = &text[start..index];
        if is_year_like(code) {
            continue;
        }

        let score = score_run(code, start, &keyword_positions);
        if best
            .as_ref()
            .is_none_or(|(best_score, _)| score > *best_score)
        {
            best = Some((score, code.to_string()));
        }
    }

    best.filter(|(score, _)| *score >= min_score)
        .map(|(_, code)| code)
}

fn find_keywords(text: &str) -> Vec<usize> {
    let lowercase = text.to_lowercase();
    let mut positions = Vec::new();

    for keyword in KEYWORDS {
        let needle = keyword.to_lowercase();
        let mut from = 0usize;
        while from < lowercase.len() {
            match lowercase[from..].find(&needle) {
                Some(offset) => {
                    let at = from + offset;
                    positions.push(at);
                    from = at + needle.len().max(1);
                }
                None => break,
            }
        }
    }

    positions
}

fn is_year_like(code: &str) -> bool {
    code.len() == 4
        && code
            .parse::<u32>()
            .is_ok_and(|value| (1900..=2099).contains(&value))
}

fn score_run(code: &str, start: usize, keyword_positions: &[usize]) -> i32 {
    let mut score = 1;
    score += match code.len() {
        6 => 4,
        4 | 5 => 2,
        _ => 1,
    };

    let mut nearest_before = usize::MAX;
    let mut nearest_after = usize::MAX;
    for &position in keyword_positions {
        if position <= start {
            nearest_before = nearest_before.min(start - position);
        } else {
            nearest_after = nearest_after.min(position - start);
        }
    }

    if nearest_before <= KEYWORD_BONUS_BEFORE {
        score += 10;
    } else if nearest_after <= KEYWORD_BONUS_AFTER {
        score += 5;
    } else if !keyword_positions.is_empty() {
        score += 1;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 真实网易验证码短信() {
        let text = "【网易】验证码：868740，您正在登录网易手机账号（若非本人操作，请删除本短信）";
        assert_eq!(extract_otp(text).as_deref(), Some("868740"));
    }

    #[test]
    fn 真实闲鱼验证码短信() {
        let text = "【闲鱼】638706（验证码仅用于您本人进行短信验证，请勿告知他人，谨防受骗）";
        assert_eq!(extract_otp(text).as_deref(), Some("638706"));
    }

    #[test]
    fn 各种常见措辞() {
        for (text, expected) in [
            ("验证码：123456，5分钟内有效", "123456"),
            ("您的验证码是 1234", "1234"),
            ("动态码 654321 请勿泄露", "654321"),
            ("Your verification code is 483920", "483920"),
            ("OTP: 778899", "778899"),
            ("您的短信密码 12345 已发送", "12345"),
            ("安全码 998877 请妥善保管", "998877"),
        ] {
            assert_eq!(extract_otp(text).as_deref(), Some(expected), "文本: {text}");
        }
    }

    #[test]
    fn 关键词在数字之后也能提取() {
        assert_eq!(
            extract_otp("868740 是你的验证码").as_deref(),
            Some("868740")
        );
    }

    #[test]
    fn 多个候选时选最可信的那个() {
        let text = "订单 12345678 已出票。验证码：483920 请勿泄露";
        assert_eq!(extract_otp(text).as_deref(), Some("483920"));
    }

    #[test]
    fn 容易误判的内容不该提取() {
        for text in [
            "手机号 13812345678 请勿泄露",
            "会议时间 2024 年",
            "订单号 12345678 已发货",
            "没有数字的纯文本",
            "2024-09-29 的账单已生成",
            "",
            "   ",
        ] {
            assert_eq!(extract_otp(text), None, "文本: {text}");
        }
    }

    #[test]
    fn 长数字串不会被切出中间的片段() {
        assert_eq!(extract_otp("联系 13812345678"), None);
    }

    #[test]
    fn 阈值可以调高调低() {
        let text = "验证码：123456";
        assert_eq!(extract_otp(text).as_deref(), Some("123456"));
        assert_eq!(extract_otp_with_min_score(text, 100), None);
    }

    #[test]
    fn 四位验证码在关键词旁边也能识别() {
        assert_eq!(extract_otp("验证码 1234").as_deref(), Some("1234"));
    }

    #[test]
    fn 四位年份不会被当成验证码() {
        assert_eq!(extract_otp("验证码有效期至 2024"), None);
        assert_eq!(extract_otp("请于 1998 年前办理"), None);
    }

    #[test]
    fn 英文大小写都能命中关键词() {
        assert_eq!(
            extract_otp("Your Verification Code is 483920").as_deref(),
            Some("483920")
        );
        assert_eq!(
            extract_otp("SECURITY CODE: 556677").as_deref(),
            Some("556677")
        );
    }

    #[test]
    fn 八位验证码也在范围内() {
        assert_eq!(extract_otp("验证码：12345678").as_deref(), Some("12345678"));
    }

    #[test]
    fn 九位数字不算验证码() {
        assert_eq!(extract_otp("验证码：123456789"), None);
    }

    #[test]
    fn extractor_trait_uses_threshold() {
        let ext = DefaultOtpExtractor::default();
        assert_eq!(
            ext.extract("【网易】验证码：868740").as_deref(),
            Some("868740")
        );
    }
}

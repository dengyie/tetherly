// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Percent-encoding/decoding and query-string parsing shared by the router and
//! the loopback client helpers.

pub(super) fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let src = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        if b == b'%' {
            // Only a full `%XX` is an escape; anything else stays literal and
            // the following bytes are decoded normally (never swallowed).
            if let (Some(h), Some(l)) = (
                src.get(i + 1).copied().and_then(hex),
                src.get(i + 2).copied().and_then(hex),
            ) {
                bytes.push(h << 4 | l);
                i += 3;
                continue;
            }
        } else if b == b'+' {
            bytes.push(b' ');
            i += 1;
            continue;
        }
        bytes.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).to_string()
}

pub fn urlencode_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub(super) fn parse_query_map(q: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(percent_decode(k), percent_decode(v));
        } else {
            map.insert(percent_decode(pair), String::new());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_percent_escapes_keep_every_byte() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
        assert_eq!(percent_decode("50%-off"), "50%-off");
        assert_eq!(percent_decode("x%2"), "x%2");
        assert_eq!(percent_decode("%%41"), "%A");
        assert_eq!(percent_decode("a%20b+c"), "a b c");
    }
}

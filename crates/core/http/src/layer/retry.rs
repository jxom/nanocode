use std::time::Duration;

use crate::types::{HeaderMap, HeaderName};

const SHOULD_RETRY: HeaderName = HeaderName::from_static("x-should-retry");

const RETRY_AFTER_MS: HeaderName = HeaderName::from_static("retry-after-ms");

#[must_use]
pub fn should_retry(headers: &HeaderMap) -> Option<bool> {
    match headers.get(SHOULD_RETRY)?.as_bytes() {
        b"true" => Some(true),
        b"false" => Some(false),
        _ => None,
    }
}

pub const PATIENCE: Duration = Duration::from_secs(600);

#[must_use]
pub const fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500..=599)
}

#[must_use]
pub const fn patient_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504 | 529)
}

#[must_use]
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    retry_after_ms(headers).or_else(|| {
        let seconds = headers
            .get(crate::types::header::RETRY_AFTER)?
            .to_str()
            .ok()?
            .trim()
            .parse()
            .ok()?;
        Some(Duration::from_secs(seconds))
    })
}

#[must_use]
pub fn retry_after_ms(headers: &HeaderMap) -> Option<Duration> {
    let text = headers.get(RETRY_AFTER_MS)?.to_str().ok()?.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let nanos = fraction
        .bytes()
        .chain(std::iter::repeat(b'0'))
        .take(6)
        .fold(0, |nanos, digit| nanos * 10 + u64::from(digit - b'0'));
    Duration::from_millis(whole.parse().ok()?).checked_add(Duration::from_nanos(nanos))
}

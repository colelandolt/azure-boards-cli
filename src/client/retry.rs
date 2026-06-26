use std::time::Duration;

pub const MAX_ATTEMPTS: u32 = 3; // retries after the first try (4 total)

/// Parse Retry-After (delta-seconds or HTTP-date) and retry-after-ms.
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    if let Some(ms) = headers
        .get("retry-after-ms")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return Some(Duration::from_millis(ms));
    }
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(secs) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    // HTTP-date form.
    let date =
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc2822).ok()?;
    let delta = date - time::OffsetDateTime::now_utc();
    Some(delta.try_into().unwrap_or(Duration::ZERO))
}

/// Server hint wins; otherwise exponential backoff with jitter, capped at 30s.
pub fn delay(attempt: u32, retry_after: Option<Duration>) -> Duration {
    if let Some(d) = retry_after {
        return d.min(Duration::from_secs(60));
    }
    let base = Duration::from_millis(500).saturating_mul(2u32.saturating_pow(attempt));
    let jitter = Duration::from_millis(fastrand_jitter());
    (base + jitter).min(Duration::from_secs(30))
}

/// Cheap jitter (0..250ms) without a rand dependency.
fn fastrand_jitter() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % 250)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_parsed() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "7".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(7)));
    }

    #[test]
    fn retry_after_ms_preferred() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "7".parse().unwrap());
        headers.insert("retry-after-ms", "1500".parse().unwrap());
        assert_eq!(
            parse_retry_after(&headers),
            Some(Duration::from_millis(1500))
        );
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert!(delay(0, None) >= Duration::from_millis(500));
        assert!(delay(0, None) < Duration::from_secs(1));
        assert!(delay(10, None) <= Duration::from_secs(30));
        assert_eq!(
            delay(0, Some(Duration::from_secs(9))),
            Duration::from_secs(9)
        );
    }
}

//! Native authority maps clock failure to an explicit stale-access result.
use super::*;
pub(super) fn elapsed() -> Result<Duration, RemoteError> {
    qgenie_remote_transport::elapsed::now().map_err(|_| {
        RemoteError::rejected(
            ErrorCode::AccessStale,
            "continuous native authority clock is unavailable",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_continuous_clock_is_available_and_monotonic() {
        let before = elapsed().unwrap();
        thread::sleep(Duration::from_millis(2));
        assert!(elapsed().unwrap() >= before);
    }
}

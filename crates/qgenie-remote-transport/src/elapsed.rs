//! A process-independent elapsed clock that counts suspension. No policy or retries.
use std::{io, sync::Mutex, time::Duration};
#[derive(Default)]
struct Series {
    last: Option<Duration>,
    failed: bool,
}
impl Series {
    fn observe(&mut self, value: Option<Duration>) -> io::Result<Duration> {
        if self.failed {
            return Err(unavailable());
        }
        let Some(value) = value.filter(|value| self.last.is_none_or(|last| *value >= last)) else {
            self.failed = true;
            return Err(unavailable());
        };
        self.last = Some(value);
        Ok(value)
    }
}
fn unavailable() -> io::Error {
    io::Error::other("continuous elapsed clock failed; process restart required")
}
/// Time since boot, including sleep. A failed/backward clock stays unavailable
/// for this process, so an old deadline cannot revive when its counter catches up.
pub fn now() -> io::Result<Duration> {
    static SERIES: Mutex<Series> = Mutex::new(Series {
        last: None,
        failed: false,
    });
    let mut series = SERIES.lock().map_err(|_| unavailable())?;
    // Serialize sampling too: concurrent callers returning in the opposite order
    // must not be mistaken for a backward kernel clock.
    series.observe(platform())
}
#[cfg(target_os = "linux")]
fn platform() -> Option<Duration> {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: a live, correctly sized output pointer and Linux's suspend-aware clock.
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, time.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: successful clock_gettime initialized the complete timespec.
    let time = unsafe { time.assume_init() };
    let nanos = u32::try_from(time.tv_nsec).ok()?;
    if nanos >= 1_000_000_000 {
        return None;
    }
    Some(Duration::new(u64::try_from(time.tv_sec).ok()?, nanos))
}
#[cfg(target_os = "macos")]
fn platform() -> Option<Duration> {
    // Available since macOS 10.12; unlike mach_absolute_time this includes sleep.
    unsafe extern "C" {
        fn mach_continuous_time() -> u64;
    }
    static FACTOR: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    let &(numer, denom) = FACTOR
        .get_or_init(|| {
            let mut info = std::mem::MaybeUninit::<libc::mach_timebase_info>::uninit();
            // SAFETY: the kernel writes a complete timebase into this output pointer.
            if unsafe { libc::mach_timebase_info(info.as_mut_ptr()) } != 0 {
                return None;
            }
            // SAFETY: successful mach_timebase_info initialized both fields.
            let info = unsafe { info.assume_init() };
            if info.numer == 0 || info.denom == 0 {
                None
            } else {
                Some((info.numer, info.denom))
            }
        })
        .as_ref()?;
    // SAFETY: this takes no arguments and returns a process-independent kernel tick.
    let ticks = unsafe { mach_continuous_time() };
    let nanos = u128::from(ticks) * u128::from(numer) / u128::from(denom);
    Some(Duration::new(
        u64::try_from(nanos / 1_000_000_000).ok()?,
        (nanos % 1_000_000_000) as u32,
    ))
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform() -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_and_regression_cannot_revive_old_deadlines() {
        let mut series = Series::default();
        series.observe(Some(Duration::from_secs(100))).unwrap();
        assert!(series.observe(Some(Duration::from_secs(99))).is_err());
        assert!(series.observe(Some(Duration::from_secs(110))).is_err());
        let mut series = Series::default();
        assert!(series.observe(None).is_err());
        assert!(series.observe(Some(Duration::from_secs(1000))).is_err());
        assert!(
            Series::default()
                .observe(Some(Duration::from_secs(1000)))
                .is_ok()
        );
    }
}

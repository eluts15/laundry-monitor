use core::task::Waker;

use esp_hal::time::{Duration, Instant};

pub fn noop_waker() -> Waker {
    Waker::noop().clone() // stable as of Rust 1.85 / noop_waker feature
}

/// Busy-wait delay. Appropriate for a single-task no-OS design.
pub fn blocking_delay(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {}
}

/// Parses a single IPv4 octet from a decimal string at compile time.
pub const fn octet(s: &str) -> u8 {
    match s.as_bytes() {
        [a] => *a - b'0',
        [a, b] => (*a - b'0') * 10 + (*b - b'0'),
        [a, b, c] => (*a - b'0') * 100 + (*b - b'0') * 10 + (*c - b'0'),
        _ => panic!("invalid octet"),
    }
}

// TODO: These functions should be generic.
/// Parses a `u16` from a decimal string at compile time.
pub const fn parse_u16(s: &str) -> u16 {
    let s = s.as_bytes();
    let mut val: u16 = 0;
    let mut i = 0;
    while i < s.len() {
        val = val * 10 + (s[i] - b'0') as u16;
        i += 1;
    }
    val
}

/// Parses a `u64` from a decimal string at compile time.
/// Used for idle timeout values supplied via `.env`.
pub const fn parse_u64(s: &str) -> u64 {
    let s = s.as_bytes();
    let mut val: u64 = 0;
    let mut i = 0;
    while i < s.len() {
        val = val * 10 + (s[i] - b'0') as u64;
        i += 1;
    }
    val
}

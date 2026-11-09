//! Reconnect delays, as the mobile app's `reconnectingSocket` (`apps/mobile/src/data/socket.ts`):
//! 1×, 2×, 4× … up to 30× the base, each plus up to half of it at random, so clients that lost
//! the server together (a restart closes every socket with 1012) do not come back together.

use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Backoff {
    base: Duration,
    attempt: u32,
    seed: u64,
}

impl Backoff {
    pub fn new(base: Duration) -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
        Backoff { base, attempt: 0, seed }
    }

    /// A connection held: the next drop starts from the base again.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The wait before the next try.
    pub fn next_delay(&mut self) -> Duration {
        let jitter = self.random();
        self.delay_with(jitter)
    }

    /// The wait with `jitter` in `0.0..1.0`.
    pub fn delay_with(&mut self, jitter: f64) -> Duration {
        let factor = 2u32.saturating_pow(self.attempt.min(16)).min(30);
        self.attempt = self.attempt.saturating_add(1);
        self.base.mul_f64(f64::from(factor) * (1.0 + jitter.clamp(0.0, 1.0) / 2.0))
    }

    /// xorshift64: enough to spread clients apart, no dependency.
    fn random(&mut self) -> f64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_double_up_to_thirty_times_the_base_and_reset_on_open() {
        let mut b = Backoff::new(Duration::from_secs(1));
        let secs: Vec<u64> = (0..7).map(|_| b.delay_with(0.0).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30]);
        b.reset();
        assert_eq!(b.delay_with(1.0), Duration::from_millis(1500), "jitter adds up to half");
    }

    #[test]
    fn random_jitter_stays_in_range_and_varies() {
        let mut b = Backoff::new(Duration::from_millis(100));
        let delays: Vec<Duration> = (0..50).map(|_| { b.reset(); b.next_delay() }).collect();
        assert!(delays.iter().all(|d| *d >= Duration::from_millis(100) && *d <= Duration::from_millis(150)));
        assert!(delays.iter().any(|d| *d != delays[0]), "not all the same");
    }
}

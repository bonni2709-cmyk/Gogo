//! Retry scheduling without network I/O, independently testable with a synthetic clock.
use std::time::{Duration, Instant};

const INITIAL: Duration = Duration::from_secs(5);
const MAXIMUM: Duration = Duration::from_secs(60);
const VERIFIED_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct RecoveryPolicy {
    next_delay: Duration,
    retry_at: Option<Instant>,
    last_verified: Option<Instant>,
}
impl Default for RecoveryPolicy {
    fn default() -> Self { Self { next_delay: INITIAL, retry_at: None, last_verified: None } }
}
impl RecoveryPolicy {
    /// Duplicate failure signals do not postpone an already scheduled retry.
    pub fn failed(&mut self, now: Instant) -> Instant {
        if let Some(at) = self.retry_at { return at; }
        let mut at = now + self.next_delay;
        if let Some(last) = self.last_verified { at = at.max(last + VERIFIED_COOLDOWN); }
        self.retry_at = Some(at);
        self.next_delay = (self.next_delay * 2).min(MAXIMUM);
        at
    }
    pub fn retry_at(&self) -> Option<Instant> { self.retry_at }
    pub fn begin_due_attempt(&mut self, now: Instant) -> bool {
        if self.retry_at.is_some_and(|at| at <= now) {
            self.retry_at = None;
            true
        } else { false }
    }
    pub fn verified(&mut self, now: Instant) {
        self.retry_at = None;
        self.next_delay = INITIAL;
        self.last_verified = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exponential_backoff_is_bounded() {
        let mut policy = RecoveryPolicy::default();
        let mut now = Instant::now();
        for seconds in [5, 10, 20, 40, 60, 60] {
            let at = policy.failed(now);
            assert_eq!(at.duration_since(now), Duration::from_secs(seconds));
            assert!(!policy.begin_due_attempt(now));
            assert!(policy.begin_due_attempt(at));
            now = at;
        }
    }
    #[test]
    fn duplicate_signals_do_not_reset_deadline_or_increase_backoff() {
        let mut policy = RecoveryPolicy::default();
        let now = Instant::now();
        let at = policy.failed(now);
        assert_eq!(policy.failed(now + Duration::from_secs(3)), at);
        assert!(policy.begin_due_attempt(at));
        assert_eq!(policy.failed(at), at + Duration::from_secs(10));
    }
    #[test]
    fn verified_transport_has_a_recovery_cooldown() {
        let mut policy = RecoveryPolicy::default();
        let now = Instant::now();
        policy.verified(now);
        let at = policy.failed(now + Duration::from_secs(1));
        assert_eq!(at, now + Duration::from_secs(30));
        assert!(policy.begin_due_attempt(at));
        policy.verified(at);
        assert_eq!(policy.retry_at(), None);
        // After the cooldown has passed, success has reset the delay to five seconds.
        assert_eq!(policy.failed(at + Duration::from_secs(40)), at + Duration::from_secs(45));
    }
}

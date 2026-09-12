// SPDX-License-Identifier: GPL-3.0-or-later
//! Slowing down password guessing.
//!
//! The Web UI authenticates with one shared password and issues an admin
//! session, so an online guess is the whole attack. scrypt already makes each
//! attempt cost something; this makes a run of them cost time as well.
//!
//! The shape is a token bucket per client address, refilled steadily. A few
//! attempts in a row are free, which is what a person who mistypes needs, and
//! sustained guessing settles at one attempt every few seconds however many
//! addresses it comes from, because each one is limited on its own.
//!
//! It is not a defence against a distributed attack, and it cannot be: the
//! server cannot tell a thousand hosts guessing once from a thousand people
//! logging in. It is a defence against the single host that tries all night.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Attempts allowed in a burst before the wait starts.
pub const BURST: u32 = 5;

/// How long one attempt takes to come back.
///
/// Thirty seconds rather than a few, because verifying a password already
/// costs a second or so of scrypt: a refill near that cost would hand a token
/// back at about the rate attempts consume them, and the bucket would never
/// empty. The limit has to be slower than the work it is limiting.
pub const REFILL: Duration = Duration::from_secs(30);

/// An address that has not been seen for this long is forgotten, so the map
/// does not grow with every client that ever visited.
const FORGET_AFTER: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy)]
struct Bucket {
    /// Attempts left, in thousandths so the refill does not lose fractions.
    tokens: u32,
    seen: Instant,
}

const SCALE: u32 = 1000;

/// Failed-login budgets, one per client address.
#[derive(Debug, Default)]
pub struct Throttle {
    buckets: HashMap<String, Bucket>,
}

impl Throttle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this client may try, and how long until it may if not.
    ///
    /// Only failures spend a token, so someone who logs in correctly is never
    /// held up, however many tabs they open.
    pub fn check(&mut self, client: &str, now: Instant) -> Result<(), Duration> {
        let bucket = self.refill(client, now);
        if bucket.tokens >= SCALE {
            Ok(())
        } else {
            let missing = SCALE - bucket.tokens;
            Err(REFILL.mul_f64(f64::from(missing) / f64::from(SCALE)))
        }
    }

    /// Records a failed attempt.
    pub fn failed(&mut self, client: &str, now: Instant) {
        let bucket = self.refill(client, now);
        bucket.tokens = bucket.tokens.saturating_sub(SCALE);
    }

    /// Forgets a client, which a successful login does: a correct password is
    /// proof this was not an attack.
    pub fn succeeded(&mut self, client: &str) {
        self.buckets.remove(client);
    }

    /// Drops clients nothing has been heard from in an hour.
    pub fn sweep(&mut self, now: Instant) -> usize {
        let before = self.buckets.len();
        self.buckets
            .retain(|_, bucket| now.duration_since(bucket.seen) < FORGET_AFTER);
        before - self.buckets.len()
    }

    pub fn tracked(&self) -> usize {
        self.buckets.len()
    }

    fn refill(&mut self, client: &str, now: Instant) -> &mut Bucket {
        let bucket = self.buckets.entry(client.to_owned()).or_insert(Bucket {
            tokens: BURST * SCALE,
            seen: now,
        });

        let elapsed = now.saturating_duration_since(bucket.seen);
        if elapsed > Duration::ZERO {
            let gained = (elapsed.as_secs_f64() / REFILL.as_secs_f64() * f64::from(SCALE)) as u64;
            let gained = u32::try_from(gained).unwrap_or(u32::MAX);
            bucket.tokens = bucket.tokens.saturating_add(gained).min(BURST * SCALE);
            bucket.seen = now;
        }
        bucket
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_client_may_try() {
        let mut throttle = Throttle::new();
        assert!(throttle.check("1.2.3.4", Instant::now()).is_ok());
    }

    #[test]
    fn a_burst_of_wrong_passwords_is_allowed_before_the_wait_starts() {
        // Someone who mistypes a few times is not an attack.
        let mut throttle = Throttle::new();
        let now = Instant::now();

        for _ in 0..BURST {
            assert!(throttle.check("1.2.3.4", now).is_ok());
            throttle.failed("1.2.3.4", now);
        }
        assert!(throttle.check("1.2.3.4", now).is_err());
    }

    #[test]
    fn the_refusal_says_how_long_to_wait() {
        let mut throttle = Throttle::new();
        let now = Instant::now();
        for _ in 0..BURST {
            throttle.failed("1.2.3.4", now);
        }

        let wait = throttle.check("1.2.3.4", now).unwrap_err();
        assert!(wait <= REFILL, "{wait:?}");
        assert!(wait > Duration::ZERO);
    }

    #[test]
    fn waiting_earns_another_attempt() {
        let mut throttle = Throttle::new();
        let now = Instant::now();
        for _ in 0..BURST {
            throttle.failed("1.2.3.4", now);
        }
        assert!(throttle.check("1.2.3.4", now).is_err());

        let later = now + REFILL;
        assert!(throttle.check("1.2.3.4", later).is_ok());
    }

    #[test]
    fn the_budget_does_not_grow_past_the_burst() {
        // A client idle for a day must not bank a day of attempts.
        let mut throttle = Throttle::new();
        let now = Instant::now();
        throttle.failed("1.2.3.4", now);

        let much_later = now + Duration::from_secs(86_400);
        assert!(throttle.check("1.2.3.4", much_later).is_ok());
        for _ in 0..BURST {
            throttle.failed("1.2.3.4", much_later);
        }
        assert!(throttle.check("1.2.3.4", much_later).is_err());
    }

    #[test]
    fn one_client_being_throttled_does_not_hold_up_another() {
        let mut throttle = Throttle::new();
        let now = Instant::now();
        for _ in 0..BURST {
            throttle.failed("1.2.3.4", now);
        }

        assert!(throttle.check("1.2.3.4", now).is_err());
        assert!(throttle.check("5.6.7.8", now).is_ok());
    }

    #[test]
    fn a_correct_password_clears_the_record() {
        let mut throttle = Throttle::new();
        let now = Instant::now();
        for _ in 0..BURST - 1 {
            throttle.failed("1.2.3.4", now);
        }

        throttle.succeeded("1.2.3.4");
        assert_eq!(throttle.tracked(), 0);
        for _ in 0..BURST {
            assert!(throttle.check("1.2.3.4", now).is_ok());
            throttle.failed("1.2.3.4", now);
        }
    }

    #[test]
    fn clients_are_forgotten_once_they_stop_coming_back() {
        // Otherwise the map grows by one entry per address, for ever.
        let mut throttle = Throttle::new();
        let now = Instant::now();
        throttle.failed("1.2.3.4", now);
        throttle.failed("5.6.7.8", now);
        assert_eq!(throttle.tracked(), 2);

        assert_eq!(throttle.sweep(now), 0, "nothing is stale yet");
        assert_eq!(throttle.sweep(now + FORGET_AFTER + REFILL), 2);
        assert_eq!(throttle.tracked(), 0);
    }
}

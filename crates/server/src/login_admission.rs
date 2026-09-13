use std::{
    collections::HashMap,
    hash::Hash,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::error::AppError;
use sha2::{Digest, Sha256};

pub(crate) const LOGIN_BODY_LIMIT_BYTES: usize = 4 * 1024;

const SOURCE_BURST: u32 = 16;
const SOURCE_REFILL_INTERVAL: Duration = Duration::from_secs(1);
const SOURCE_ENTRY_CAPACITY: usize = 2_048;
const ACCOUNT_BURST: u32 = 8;
const ACCOUNT_REFILL_INTERVAL: Duration = Duration::from_secs(15);
const ACCOUNT_ENTRY_CAPACITY: usize = 4_096;
const RATE_ENTRY_TTL: Duration = Duration::from_secs(15 * 60);
#[derive(Clone)]
pub(crate) struct LoginAdmission {
    rates: Arc<Mutex<LoginRateState>>,
}

impl Default for LoginAdmission {
    fn default() -> Self {
        Self::new(
            BucketPolicy::new(SOURCE_BURST, SOURCE_REFILL_INTERVAL),
            SOURCE_ENTRY_CAPACITY,
            BucketPolicy::new(ACCOUNT_BURST, ACCOUNT_REFILL_INTERVAL),
            ACCOUNT_ENTRY_CAPACITY,
            RATE_ENTRY_TTL,
        )
    }
}

impl LoginAdmission {
    fn new(
        source_policy: BucketPolicy,
        source_capacity: usize,
        account_policy: BucketPolicy,
        account_capacity: usize,
        entry_ttl: Duration,
    ) -> Self {
        Self {
            rates: Arc::new(Mutex::new(LoginRateState {
                sources: BoundedBuckets::new(source_policy, source_capacity, entry_ttl),
                accounts: BoundedBuckets::new(account_policy, account_capacity, entry_ttl),
            })),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(source_burst: u32, account_burst: u32) -> Self {
        Self::new(
            BucketPolicy::new(source_burst, Duration::from_secs(60)),
            8,
            BucketPolicy::new(account_burst, Duration::from_secs(60)),
            8,
            Duration::from_secs(300),
        )
    }

    pub(crate) fn check_source(&self, source: IpAddr) -> Result<(), AppError> {
        self.check_source_at(canonical_ip(source), Instant::now())
    }

    fn check_source_at(&self, source: IpAddr, now: Instant) -> Result<(), AppError> {
        self.rates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sources
            .check_at(source, now)
            .map_err(rate_limited)
    }

    pub(crate) fn check_account(&self, normalized_account: &str) -> Result<(), AppError> {
        self.check_account_at(normalized_account, Instant::now())
    }

    fn check_account_at(&self, normalized_account: &str, now: Instant) -> Result<(), AppError> {
        self.rates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accounts
            .check_at(account_key(normalized_account), now)
            .map_err(rate_limited)
    }
}

fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(address)),
        IpAddr::V4(_) => address,
    }
}

fn account_key(normalized_account: &str) -> [u8; 32] {
    Sha256::digest(normalized_account.as_bytes()).into()
}

fn rate_limited(delay: Duration) -> AppError {
    AppError::too_many_requests(retry_after_seconds(delay))
}

fn retry_after_seconds(delay: Duration) -> u64 {
    delay
        .as_secs()
        .saturating_add(u64::from(delay.subsec_nanos() != 0))
        .max(1)
}

struct LoginRateState {
    sources: BoundedBuckets<IpAddr>,
    accounts: BoundedBuckets<[u8; 32]>,
}

#[derive(Clone, Copy)]
struct BucketPolicy {
    burst: u32,
    refill_interval: Duration,
}

impl BucketPolicy {
    fn new(burst: u32, refill_interval: Duration) -> Self {
        assert!(burst > 0);
        assert!(!refill_interval.is_zero());
        Self {
            burst,
            refill_interval,
        }
    }
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
    last_seen: Instant,
}

impl Bucket {
    fn new(policy: BucketPolicy, now: Instant) -> Self {
        Self {
            tokens: f64::from(policy.burst),
            last_refill: now,
            last_seen: now,
        }
    }

    fn tokens_at(self, policy: BucketPolicy, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.last_refill);
        (self.tokens + elapsed.as_secs_f64() / policy.refill_interval.as_secs_f64())
            .min(f64::from(policy.burst))
    }

    fn refill(&mut self, policy: BucketPolicy, now: Instant) {
        self.tokens = self.tokens_at(policy, now);
        self.last_refill = now;
        self.last_seen = now;
    }

    fn delay_until_tokens(self, policy: BucketPolicy, wanted: f64, now: Instant) -> Duration {
        let missing = (wanted - self.tokens_at(policy, now)).max(0.0);
        Duration::from_secs_f64(missing * policy.refill_interval.as_secs_f64())
    }
}

struct BoundedBuckets<K> {
    entries: HashMap<K, Bucket>,
    policy: BucketPolicy,
    capacity: usize,
    entry_ttl: Duration,
}

impl<K> BoundedBuckets<K>
where
    K: Clone + Eq + Hash,
{
    fn new(policy: BucketPolicy, capacity: usize, entry_ttl: Duration) -> Self {
        assert!(capacity > 0);
        assert!(!entry_ttl.is_zero());
        Self {
            entries: HashMap::new(),
            policy,
            capacity,
            entry_ttl,
        }
    }

    fn check_at(&mut self, key: K, now: Instant) -> Result<(), Duration> {
        self.prune_expired(now);
        if !self.entries.contains_key(&key) {
            self.make_room(now)?;
            self.entries
                .insert(key.clone(), Bucket::new(self.policy, now));
        }
        let entry = self.entries.get_mut(&key).expect("bucket was inserted");
        entry.refill(self.policy, now);
        if entry.tokens < 1.0 {
            return Err(entry.delay_until_tokens(self.policy, 1.0, now));
        }
        entry.tokens -= 1.0;
        Ok(())
    }

    fn prune_expired(&mut self, now: Instant) {
        self.entries
            .retain(|_, entry| now.saturating_duration_since(entry.last_seen) < self.entry_ttl);
    }

    fn make_room(&mut self, now: Instant) -> Result<(), Duration> {
        if self.entries.len() < self.capacity {
            return Ok(());
        }
        let evictable = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.tokens_at(self.policy, now) >= f64::from(self.policy.burst))
            .min_by_key(|(_, entry)| entry.last_seen)
            .map(|(key, _)| key.clone());
        if let Some(key) = evictable {
            self.entries.remove(&key);
            return Ok(());
        }
        let retry = self
            .entries
            .values()
            .map(|entry| {
                let until_full =
                    entry.delay_until_tokens(self.policy, f64::from(self.policy.burst), now);
                let until_expiry = self
                    .entry_ttl
                    .saturating_sub(now.saturating_duration_since(entry.last_seen));
                until_full.min(until_expiry)
            })
            .min()
            .unwrap_or(self.entry_ttl);
        Err(retry.max(Duration::from_nanos(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_normalized_account_budgets_are_independent() {
        let admission = LoginAdmission::for_test(2, 2);
        let now = Instant::now();
        let source: IpAddr = "192.0.2.10".parse().unwrap();
        admission.check_source_at(source, now).unwrap();
        admission.check_source_at(source, now).unwrap();
        assert!(admission.check_source_at(source, now).is_err());
        admission
            .check_source_at("192.0.2.11".parse().unwrap(), now)
            .unwrap();

        admission
            .check_account_at("admin@example.com", now)
            .unwrap();
        admission
            .check_account_at("admin@example.com", now)
            .unwrap();
        assert!(admission
            .check_account_at("admin@example.com", now)
            .is_err());
        admission
            .check_account_at("other@example.com", now)
            .unwrap();
    }

    #[test]
    fn bounded_buckets_expire_and_only_evict_replenished_entries() {
        let policy = BucketPolicy::new(1, Duration::from_secs(1));
        let ttl = Duration::from_secs(10);
        let mut buckets = BoundedBuckets::new(policy, 2, ttl);
        let now = Instant::now();
        buckets.check_at("first", now).unwrap();
        buckets
            .check_at("second", now + Duration::from_millis(100))
            .unwrap();
        assert!(buckets
            .check_at("third", now + Duration::from_millis(200))
            .is_err());
        buckets
            .check_at("third", now + Duration::from_secs(2))
            .unwrap();
        assert!(!buckets.entries.contains_key("first"));
        buckets
            .check_at("after-expiry", now + ttl + Duration::from_secs(3))
            .unwrap();
        assert_eq!(buckets.entries.len(), 1);
    }
}

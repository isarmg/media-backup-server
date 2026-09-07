use std::{
    collections::HashMap,
    hash::Hash,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::error::AppError;

pub(crate) const LOGIN_BODY_LIMIT_BYTES: usize = 4 * 1024;

const SOURCE_BURST: u32 = 16;
const SOURCE_REFILL_INTERVAL: Duration = Duration::from_secs(1);
const SOURCE_ENTRY_CAPACITY: usize = 2_048;
const ACCOUNT_BURST: u32 = 8;
const ACCOUNT_REFILL_INTERVAL: Duration = Duration::from_secs(15);
const ACCOUNT_ENTRY_CAPACITY: usize = 4_096;
const RATE_ENTRY_TTL: Duration = Duration::from_secs(15 * 60);
const ARGON2_CONCURRENCY: usize = 2;
const ARGON2_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(1);
const ARGON2_EXECUTION_TIMEOUT: Duration = Duration::from_secs(10);
// Generated with the Foundation 0.3 current Argon2id policy. It is never
// accepted as a real credential.
const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$aOVS660TGXMspMgoSOcv6A$1eMydp1lX0/SzNdUYR28nln2fa2gMPGF626+W6gPKK8";

#[derive(Clone)]
pub(crate) struct LoginAdmission {
    rates: Arc<Mutex<LoginRateState>>,
    argon2_slots: Arc<Semaphore>,
    argon2_acquire_timeout: Duration,
    argon2_execution_timeout: Duration,
    dummy_password_hash: Arc<str>,
}

impl Default for LoginAdmission {
    fn default() -> Self {
        Self::new(
            BucketPolicy::new(SOURCE_BURST, SOURCE_REFILL_INTERVAL),
            SOURCE_ENTRY_CAPACITY,
            BucketPolicy::new(ACCOUNT_BURST, ACCOUNT_REFILL_INTERVAL),
            ACCOUNT_ENTRY_CAPACITY,
            RATE_ENTRY_TTL,
            ARGON2_CONCURRENCY,
            ARGON2_ACQUIRE_TIMEOUT,
            ARGON2_EXECUTION_TIMEOUT,
        )
    }
}

impl LoginAdmission {
    #[allow(clippy::too_many_arguments)]
    fn new(
        source_policy: BucketPolicy,
        source_capacity: usize,
        account_policy: BucketPolicy,
        account_capacity: usize,
        entry_ttl: Duration,
        argon2_concurrency: usize,
        argon2_acquire_timeout: Duration,
        argon2_execution_timeout: Duration,
    ) -> Self {
        assert!(argon2_concurrency > 0);
        assert!(!argon2_acquire_timeout.is_zero());
        assert!(!argon2_execution_timeout.is_zero());
        Self {
            rates: Arc::new(Mutex::new(LoginRateState {
                sources: BoundedBuckets::new(source_policy, source_capacity, entry_ttl),
                accounts: BoundedBuckets::new(account_policy, account_capacity, entry_ttl),
            })),
            argon2_slots: Arc::new(Semaphore::new(argon2_concurrency)),
            argon2_acquire_timeout,
            argon2_execution_timeout,
            dummy_password_hash: Arc::from(DUMMY_PASSWORD_HASH),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        source_burst: u32,
        account_burst: u32,
        argon2_concurrency: usize,
        argon2_acquire_timeout: Duration,
    ) -> Self {
        Self::new(
            BucketPolicy::new(source_burst, Duration::from_secs(60)),
            8,
            BucketPolicy::new(account_burst, Duration::from_secs(60)),
            8,
            Duration::from_secs(300),
            argon2_concurrency,
            argon2_acquire_timeout,
            Duration::from_secs(5),
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

    pub(crate) async fn verify(
        &self,
        password: String,
        password_hash: Option<String>,
    ) -> Result<bool, AppError> {
        let password_hash: Arc<str> = password_hash
            .map(Arc::from)
            .unwrap_or_else(|| Arc::clone(&self.dummy_password_hash));
        self.run_argon2(move || {
            crate::password::verify_current_password_blocking(&password, &password_hash)
        })
        .await
    }

    async fn run_argon2<T, F>(&self, task: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permit = match tokio::time::timeout(
            self.argon2_acquire_timeout,
            Arc::clone(&self.argon2_slots).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                return Err(AppError::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "password verification is unavailable",
                ));
            }
            Err(_) => return Err(AppError::too_many_requests(1)),
        };
        let worker = tokio::task::spawn_blocking(move || {
            // The permit stays inside the blocking task, including after an
            // HTTP request is cancelled or its execution deadline expires.
            let _permit = permit;
            task()
        });
        match tokio::time::timeout(self.argon2_execution_timeout, worker).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => {
                tracing::error!(%error, "password verification worker failed");
                Err(AppError::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "password verification is unavailable",
                ))
            }
            Err(_) => Err(AppError::too_many_requests(1)),
        }
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::oneshot;

    use super::*;

    #[test]
    fn source_and_normalized_account_budgets_are_independent() {
        let admission = LoginAdmission::for_test(2, 2, 1, Duration::from_millis(50));
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

    #[tokio::test]
    async fn unknown_user_runs_the_real_dummy_argon2_verification() {
        let admission = LoginAdmission::for_test(8, 8, 1, Duration::from_secs(1));
        assert!(!admission
            .verify("not-the-password".to_owned(), None)
            .await
            .unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn argon2_semaphore_bounds_workers_and_times_out_waiters() {
        let admission = LoginAdmission::for_test(8, 8, 1, Duration::from_millis(25));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let first_admission = admission.clone();
        let first_active = Arc::clone(&active);
        let first_maximum = Arc::clone(&maximum);
        let first = tokio::spawn(async move {
            first_admission
                .run_argon2(move || {
                    let current = first_active.fetch_add(1, Ordering::SeqCst) + 1;
                    first_maximum.fetch_max(current, Ordering::SeqCst);
                    let _ = started_tx.send(());
                    let _ = release_rx.blocking_recv();
                    first_active.fetch_sub(1, Ordering::SeqCst);
                })
                .await
        });
        started_rx.await.unwrap();
        let waiting = admission.run_argon2(|| ()).await;
        assert!(
            matches!(waiting, Err(error) if error.status == axum::http::StatusCode::TOO_MANY_REQUESTS)
        );
        release_tx.send(()).unwrap();
        first.await.unwrap().unwrap();
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
    }
}

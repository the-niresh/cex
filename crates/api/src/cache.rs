//! A read-through cache for the public history endpoints.
//!
//! `/trades` and `/candles` answer the same question for everybody. Two people
//! with the chart open want the same 200 candles, and asking the database twice
//! buys nothing. That matters more than it sounds: the history database is
//! reached over the open internet, so a query costs a fixed round trip no matter
//! how small it is, and the connection pool is only so wide. Without a cache the
//! database load grows with the number of people watching, and there is a number
//! of watchers past which the pool cannot keep up and every request queues.
//!
//! So this cache is not here to shave milliseconds. It is here to make the read
//! load **independent of how many people are looking**, which is the only shape
//! that survives being linked somewhere busy.
//!
//! Two properties do that work, and both are tested below:
//!
//! * **One loader at a time per key.** A hundred simultaneous misses produce one
//!   round trip, not a hundred. A cache without this still stampedes on a cold
//!   start, which is exactly when it is least affordable.
//! * **A stale answer beats no answer.** If the reload fails, callers keep
//!   getting the last good value until `max_stale` runs out. A chart showing
//!   candles from two seconds ago is right; a chart showing nothing because one
//!   request failed is wrong, and that blank chart is the bug this was written
//!   for. Past `max_stale` the error is reported honestly rather than papered
//!   over with data old enough to mislead.
//!
//! Only ever put public, read-only data in here. Anything scoped to one user
//! shares a process-wide map with every other user and does not belong.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Tokio's clock rather than `std`'s, so tests can advance time instead of
// sleeping through it.
use tokio::sync::RwLock;
use tokio::time::Instant;

struct Cached<V> {
    value: V,
    stored_at: Instant,
}

impl<V> Cached<V> {
    fn age(&self) -> Duration {
        self.stored_at.elapsed()
    }
}

/// One key's value, plus the lock that keeps its loaders single file.
struct Slot<V> {
    state: RwLock<Option<Cached<V>>>,
}

pub struct ReadCache<K, V> {
    /// How long a value is served without asking again.
    ttl: Duration,
    /// How far past `ttl` a value may still be served when a reload fails.
    max_stale: Duration,
    /// Ceiling on distinct keys held at once.
    capacity: usize,
    slots: Mutex<HashMap<K, Arc<Slot<V>>>>,
}

impl<K, V> ReadCache<K, V>
where
    K: Eq + Hash,
    V: Clone,
{
    pub fn new(ttl: Duration, max_stale: Duration, capacity: usize) -> Self {
        ReadCache {
            ttl,
            max_stale,
            capacity,
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// The cached value for `key`, loading it if what we have is too old.
    ///
    /// `load` runs at most once per key at a time. Errors are never cached — but
    /// a previous value may be served in their place, see the module note.
    pub async fn get_or_load<F, Fut, E>(&self, key: K, load: F) -> Result<V, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        let slot = self.slot(key);

        // The common case, and the reason for a read-write lock rather than a
        // plain one: a fresh value is handed to every caller at once.
        if let Some(fresh) = self.fresh(&*slot.state.read().await) {
            return Ok(fresh);
        }

        // Stale or missing. Taking the write lock is what makes the loaders
        // single file; everyone behind it waits for the one round trip.
        let mut state = slot.state.write().await;

        // Somebody may have filled it while we queued for the lock: read locks
        // are shared, so two callers can both find it stale and both come here.
        // Rare, because there is no yield between the read above and the write
        // here — rare enough that no test below manages to schedule it, which is
        // why this line is a guard rather than something with a test of its own.
        // It costs one comparison, and without it that pair makes two round
        // trips where one would do.
        if let Some(fresh) = self.fresh(&state) {
            return Ok(fresh);
        }

        match load().await {
            Ok(value) => {
                *state = Some(Cached {
                    value: value.clone(),
                    stored_at: Instant::now(),
                });
                Ok(value)
            }
            // Keep the stale entry rather than clearing it: if the database is
            // down, the next caller a moment from now should still get the last
            // good answer instead of starting from nothing.
            Err(e) => match state.as_ref() {
                Some(stale) if stale.age() <= self.max_stale => Ok(stale.value.clone()),
                _ => Err(e),
            },
        }
    }

    fn fresh(&self, state: &Option<Cached<V>>) -> Option<V> {
        state
            .as_ref()
            .filter(|c| c.age() < self.ttl)
            .map(|c| c.value.clone())
    }

    fn slot(&self, key: K) -> Arc<Slot<V>> {
        // A poisoned lock here would mean a panic while doing nothing but map
        // lookups. Carrying on with the map is better than turning one panic
        // into every later request panicking too.
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(slot) = slots.get(&key) {
            return Arc::clone(slot);
        }

        // The key comes from the query string, so an unbounded map is an
        // unbounded allocation for anyone who can vary it. Real traffic uses a
        // handful of keys and never reaches this; a scan for distinct keys hits
        // it immediately and simply loses the cache rather than the process.
        if slots.len() >= self.capacity {
            slots.clear();
        }

        let slot = Arc::new(Slot {
            state: RwLock::new(None),
        });
        slots.insert(key, Arc::clone(&slot));
        slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts how many times the loader actually ran, which is the number every
    /// test here is really about.
    struct Loader {
        calls: AtomicUsize,
    }

    impl Loader {
        fn new() -> Self {
            Loader {
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        async fn ok(&self, value: &str) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(value.to_string())
        }

        async fn fail(&self) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err("database is unreachable".to_string())
        }
    }

    fn cache() -> ReadCache<String, String> {
        ReadCache::new(Duration::from_secs(2), Duration::from_secs(60), 64)
    }

    #[tokio::test(start_paused = true)]
    async fn serves_a_second_caller_without_loading_again() {
        let cache = cache();
        let loader = Loader::new();

        let first = cache.get_or_load("k".into(), || loader.ok("v")).await;
        let second = cache.get_or_load("k".into(), || loader.ok("v")).await;

        assert_eq!(first, Ok("v".to_string()));
        assert_eq!(second, Ok("v".to_string()));
        assert_eq!(
            loader.calls(),
            1,
            "the second caller should have been served from the cache"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn loads_again_once_the_value_has_expired() {
        let cache = cache();
        let loader = Loader::new();

        cache
            .get_or_load("k".into(), || loader.ok("old"))
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(3)).await;
        let after = cache.get_or_load("k".into(), || loader.ok("new")).await;

        assert_eq!(after, Ok("new".to_string()));
        assert_eq!(loader.calls(), 2);
    }

    /// The property the whole module exists for: a crowd arriving at a cold
    /// cache costs one round trip, not one each.
    #[tokio::test(start_paused = true)]
    async fn a_crowd_of_simultaneous_misses_loads_exactly_once() {
        let cache = Arc::new(cache());
        let loader = Arc::new(Loader::new());

        let mut handles = Vec::new();
        for _ in 0..50 {
            let cache = Arc::clone(&cache);
            let loader = Arc::clone(&loader);
            handles.push(tokio::spawn(async move {
                cache
                    .get_or_load("k".to_string(), || async {
                        // A real load is not instant, which is precisely why the
                        // others pile up behind it.
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        loader.ok("v").await
                    })
                    .await
            }));
        }

        for handle in handles {
            assert_eq!(handle.await.unwrap(), Ok("v".to_string()));
        }
        assert_eq!(
            loader.calls(),
            1,
            "50 simultaneous misses should be one round trip"
        );
    }

    /// The same guarantee as above, on real threads rather than one.
    ///
    /// The test above runs on a single-threaded runtime, where the crowd is
    /// serialised for us and the result proves less than it appears to. This one
    /// puts fifty callers on four threads behind a barrier so they arrive
    /// together, and asks for the same number back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_crowd_on_real_threads_still_loads_exactly_once() {
        let cache = Arc::new(cache());
        let loader = Arc::new(Loader::new());
        let start = tokio::sync::Barrier::new(50);
        let start = Arc::new(start);

        let mut handles = Vec::new();
        for _ in 0..50 {
            let cache = Arc::clone(&cache);
            let loader = Arc::clone(&loader);
            let start = Arc::clone(&start);
            handles.push(tokio::spawn(async move {
                // Line everybody up so they arrive together rather than in the
                // order the runtime happens to start them.
                start.wait().await;
                cache
                    .get_or_load("k".to_string(), || async {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        loader.ok("v").await
                    })
                    .await
            }));
        }

        for handle in handles {
            assert_eq!(handle.await.unwrap(), Ok("v".to_string()));
        }
        assert_eq!(
            loader.calls(),
            1,
            "a crowd on real threads should still be one round trip"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn different_keys_do_not_share_a_value() {
        let cache = cache();
        let loader = Loader::new();

        let a = cache.get_or_load("a".into(), || loader.ok("first")).await;
        let b = cache.get_or_load("b".into(), || loader.ok("second")).await;

        assert_eq!(a, Ok("first".to_string()));
        assert_eq!(b, Ok("second".to_string()));
        assert_eq!(loader.calls(), 2);
    }

    /// The blank-chart bug: one failed reload must not take the screen down.
    #[tokio::test(start_paused = true)]
    async fn serves_the_last_good_value_when_a_reload_fails() {
        let cache = cache();
        let loader = Loader::new();

        cache
            .get_or_load("k".into(), || loader.ok("good"))
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(3)).await;

        let during_outage = cache.get_or_load("k".into(), || loader.fail()).await;

        assert_eq!(during_outage, Ok("good".to_string()));
        assert_eq!(loader.calls(), 2, "it should have tried, then fallen back");
    }

    #[tokio::test(start_paused = true)]
    async fn stops_serving_a_value_that_has_gone_too_stale_to_trust() {
        let cache = cache();
        let loader = Loader::new();

        cache
            .get_or_load("k".into(), || loader.ok("good"))
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(61)).await;

        let long_outage = cache.get_or_load("k".into(), || loader.fail()).await;

        assert_eq!(long_outage, Err("database is unreachable".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn reports_the_error_when_there_is_nothing_to_fall_back_to() {
        let cache = cache();
        let loader = Loader::new();

        let cold = cache.get_or_load("k".into(), || loader.fail()).await;

        assert_eq!(cold, Err("database is unreachable".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn holds_no_more_keys_than_its_capacity() {
        let cache: ReadCache<usize, String> =
            ReadCache::new(Duration::from_secs(2), Duration::from_secs(60), 8);
        let loader = Loader::new();

        for key in 0..100 {
            cache.get_or_load(key, || loader.ok("v")).await.unwrap();
        }

        let held = cache.slots.lock().unwrap().len();
        assert!(held <= 8, "held {held} keys with a capacity of 8");
    }
}

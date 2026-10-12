//! In-memory stage cache, bounded by the bytes its artifacts hold, with least-recently-used
//! eviction.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::stage::Artifact;

/// The default budget: 512 MiB.
pub const DEFAULT_CACHE_BYTES: usize = 512 << 20;

/// What a cache holds and how often it was asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CacheStats {
    /// `get` calls that found their key.
    pub hits: u64,
    /// `get` calls that did not.
    pub misses: u64,
    /// The sum of the stored artifacts' [`Artifact::bytes`].
    pub bytes: usize,
    /// How many artifacts are stored.
    pub entries: usize,
}

struct Entry {
    value: Artifact,
    size: usize,
    /// This entry's key in `StageCache::order`.
    tick: u64,
}

/// Stage outputs by key, held within a budget of `max_bytes`.
///
/// `get` and `put` mark a key as most recently used; `contains` does not. `hits` and `misses`
/// count `get` calls and survive `clear`. Not thread safe.
pub struct StageCache {
    max_bytes: usize,
    items: HashMap<String, Entry>,
    /// Keys by the tick of their last use, so the first entry is the least recently used.
    order: BTreeMap<u64, String>,
    tick: u64,
    hits: u64,
    misses: u64,
    bytes: usize,
}

impl StageCache {
    /// An empty cache that holds at most `max_bytes`.
    pub fn new(max_bytes: usize) -> StageCache {
        StageCache {
            max_bytes,
            items: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            hits: 0,
            misses: 0,
            bytes: 0,
        }
    }

    /// The artifact stored under `key`, if any. Counts a hit or a miss and marks the key as most
    /// recently used.
    pub fn get(&mut self, key: &str) -> Option<Artifact> {
        let Some(entry) = self.items.get_mut(key) else {
            self.misses += 1;
            return None;
        };
        self.hits += 1;
        self.order.remove(&entry.tick);
        self.tick += 1;
        entry.tick = self.tick;
        self.order.insert(self.tick, key.to_string());
        Some(entry.value.clone())
    }

    /// Store `value` under `key`, replacing any old value, then evict the least recently used
    /// until the cache fits its budget.
    ///
    /// A value larger than the whole budget is not stored. The old value goes either way, so
    /// `get` never returns a value that `put` replaced.
    pub fn put(&mut self, key: &str, value: Artifact) {
        if let Some(old) = self.items.remove(key) {
            self.order.remove(&old.tick);
            self.bytes -= old.size;
        }
        let size = value.bytes();
        if size > self.max_bytes {
            return;
        }
        self.tick += 1;
        self.order.insert(self.tick, key.to_string());
        let tick = self.tick;
        self.items
            .insert(key.to_string(), Entry { value, size, tick });
        self.bytes += size;
        while self.bytes > self.max_bytes {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some(evicted) = self.items.remove(&oldest) {
                self.bytes -= evicted.size;
            }
        }
    }

    /// Whether `key` is stored. Does not count as a use.
    pub fn contains(&self, key: &str) -> bool {
        self.items.contains_key(key)
    }

    /// How many artifacts are stored.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Drop every entry. The hit and miss counts stay.
    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
        self.bytes = 0;
    }

    /// The counts and sizes so far.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            bytes: self.bytes,
            entries: self.items.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::points;

    #[test]
    fn get_put_contains_and_counts() {
        let mut c = StageCache::new(1 << 20);
        assert!(c.get("k").is_none());
        c.put("k", points(2));
        assert!(c.contains("k"));
        assert!(matches!(c.get("k"), Some(Artifact::Points(_))));
        assert_eq!(
            (c.stats().hits, c.stats().misses, c.stats().bytes),
            (1, 1, 32)
        );
        c.clear();
        assert_eq!(c.len(), 0);
        assert_eq!(c.stats().hits, 1);
    }

    #[test]
    fn evicts_least_recently_used_and_skips_oversize() {
        let mut c = StageCache::new(100); // 2 points = 32 bytes each
        c.put("a", points(2));
        c.put("b", points(2));
        c.put("c", points(2)); // 96
        c.get("a"); // b is now the oldest
        c.put("d", points(2)); // 128 > 100: evict b
        assert!(c.contains("a") && !c.contains("b") && c.contains("c") && c.contains("d"));
        c.put("big", points(10)); // 160 > 100: not stored
        assert!(!c.contains("big"));
        assert_eq!(c.stats().bytes, 96);
    }

    #[test]
    fn replacing_a_key_swaps_the_value_and_an_oversize_one_only_removes_the_old() {
        let mut c = StageCache::new(100);
        c.put("a", points(2));
        c.put("a", points(3)); // 48 bytes replace 32, not add to them
        assert_eq!((c.stats().bytes, c.stats().entries), (48, 1));
        assert!(matches!(c.get("a"), Some(Artifact::Points(p)) if p.0.len() == 3));
        c.put("a", points(10)); // 160 > 100: stays out, and the old value goes
        assert!(!c.contains("a") && c.is_empty());
        assert_eq!(c.stats().bytes, 0);
    }

    #[test]
    fn put_marks_recent_and_contains_does_not() {
        let mut c = StageCache::new(100);
        c.put("a", points(2));
        c.put("b", points(2));
        c.put("c", points(2));
        assert!(c.contains("a")); // not a use
        c.put("b", points(2)); // a use: a is now the oldest
        c.put("d", points(2));
        assert!(!c.contains("a") && c.contains("b") && c.contains("c") && c.contains("d"));
    }
}

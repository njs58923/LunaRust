// HTTP Cache implementation for scripts and fetch responses
use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, Duration};

/// Cached script with HTTP cache headers
#[derive(Debug, Clone)]
pub struct CachedScript {
    pub code: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub expires: Option<SystemTime>,
    pub cached_at: SystemTime,
}

impl CachedScript {
    pub fn new(code: String) -> Self {
        Self {
            code,
            etag: None,
            last_modified: None,
            expires: None,
            cached_at: SystemTime::now(),
        }
    }

    pub fn with_headers(
        code: String,
        etag: Option<String>,
        last_modified: Option<String>,
        cache_control: Option<&str>,
    ) -> Self {
        let expires = cache_control.and_then(|cc| {
            // Parse "max-age=3600" or similar
            cc.split(',')
                .find_map(|directive| {
                    let directive = directive.trim();
                    if directive.starts_with("max-age=") {
                        directive
                            .strip_prefix("max-age=")
                            .and_then(|s| s.parse::<u64>().ok())
                            .map(|seconds| SystemTime::now() + Duration::from_secs(seconds))
                    } else {
                        None
                    }
                })
        });

        Self {
            code,
            etag,
            last_modified,
            expires,
            cached_at: SystemTime::now(),
        }
    }

    /// Check if this cached entry is still valid
    pub fn is_valid(&self) -> bool {
        if let Some(expires) = self.expires {
            SystemTime::now() < expires
        } else {
            // If no expiry set, consider valid for 1 hour by default
            self.cached_at
                .elapsed()
                .map(|elapsed| elapsed < Duration::from_secs(3600))
                .unwrap_or(false)
        }
    }

    /// Check if we should revalidate (ETag or Last-Modified present)
    pub fn should_revalidate(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

/// LRU cache for scripts
pub struct ScriptCache {
    entries: HashMap<String, CachedScript>,
    lru: VecDeque<String>,
    max_entries: usize,
}

impl ScriptCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            max_entries,
        }
    }

    /// Get a cached script if valid
    pub fn get(&mut self, url: &str) -> Option<CachedScript> {
        if let Some(script) = self.entries.get(url) {
            if script.is_valid() {
                // Move to front of LRU
                self.lru.retain(|u| u != url);
                self.lru.push_front(url.to_string());
                return Some(script.clone());
            } else {
                // Expired, remove it
                self.entries.remove(url);
                self.lru.retain(|u| u != url);
            }
        }
        None
    }

    /// Get script for revalidation (even if expired)
    pub fn get_for_revalidation(&self, url: &str) -> Option<CachedScript> {
        self.entries.get(url).cloned()
    }

    /// Insert or update a script in the cache
    pub fn insert(&mut self, url: String, script: CachedScript) {
        // Remove if already exists (will re-add)
        self.lru.retain(|u| u != &url);

        // Add to front of LRU
        self.lru.push_front(url.clone());

        // Evict if over capacity
        while self.lru.len() > self.max_entries {
            if let Some(evicted_url) = self.lru.pop_back() {
                self.entries.remove(&evicted_url);
            }
        }

        self.entries.insert(url, script);
    }

    /// Clear all cached entries
    pub fn clear(&mut self) {
        self.entries.clear();
        self.lru.clear();
    }

    /// Get cache stats
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            total_entries: self.entries.len(),
            max_entries: self.max_entries,
            valid_entries: self.entries.values().filter(|s| s.is_valid()).count(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_entries: usize,
    pub max_entries: usize,
    pub valid_entries: usize,
}

/// Cached fetch response
#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub body: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub cached_at: SystemTime,
    pub expires: Option<SystemTime>,
}

impl CachedResponse {
    pub fn new(body: String, status: u16, headers: HashMap<String, String>) -> Self {
        let expires = headers.get("cache-control").and_then(|cc| {
            cc.split(',')
                .find_map(|directive| {
                    let directive = directive.trim();
                    if directive.starts_with("max-age=") {
                        directive
                            .strip_prefix("max-age=")
                            .and_then(|s| s.parse::<u64>().ok())
                            .map(|seconds| SystemTime::now() + Duration::from_secs(seconds))
                    } else {
                        None
                    }
                })
        });

        Self {
            body,
            status,
            headers,
            cached_at: SystemTime::now(),
            expires,
        }
    }

    pub fn is_valid(&self) -> bool {
        if let Some(expires) = self.expires {
            SystemTime::now() < expires
        } else {
            // Default: cache for 5 minutes
            self.cached_at
                .elapsed()
                .map(|elapsed| elapsed < Duration::from_secs(300))
                .unwrap_or(false)
        }
    }
}

/// LRU cache for fetch responses
pub struct FetchCache {
    entries: HashMap<String, CachedResponse>,
    lru: VecDeque<String>,
    max_entries: usize,
}

impl FetchCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            max_entries,
        }
    }

    pub fn get(&mut self, url: &str) -> Option<CachedResponse> {
        if let Some(response) = self.entries.get(url) {
            if response.is_valid() {
                self.lru.retain(|u| u != url);
                self.lru.push_front(url.to_string());
                return Some(response.clone());
            } else {
                self.entries.remove(url);
                self.lru.retain(|u| u != url);
            }
        }
        None
    }

    pub fn insert(&mut self, url: String, response: CachedResponse) {
        self.lru.retain(|u| u != &url);
        self.lru.push_front(url.clone());

        while self.lru.len() > self.max_entries {
            if let Some(evicted_url) = self.lru.pop_back() {
                self.entries.remove(&evicted_url);
            }
        }

        self.entries.insert(url, response);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.lru.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_expiry() {
        let script = CachedScript::with_headers(
            "console.log('test')".to_string(),
            None,
            None,
            Some("max-age=60"),
        );
        assert!(script.is_valid());
    }

    #[test]
    fn test_lru_eviction() {
        let mut cache = ScriptCache::new(2);
        cache.insert(
            "url1".to_string(),
            CachedScript::new("code1".to_string()),
        );
        cache.insert(
            "url2".to_string(),
            CachedScript::new("code2".to_string()),
        );
        cache.insert(
            "url3".to_string(),
            CachedScript::new("code3".to_string()),
        );

        assert_eq!(cache.entries.len(), 2);
        assert!(cache.entries.contains_key("url3"));
        assert!(cache.entries.contains_key("url2"));
        assert!(!cache.entries.contains_key("url1")); // Evicted
    }
}

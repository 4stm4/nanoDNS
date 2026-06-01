//! Простой кэш DNS-ответов без LRU, но с ограничением размера.
//!
//! Ключ: `name:qtype:qclass`. Значение: сырые байты ответа upstream + момент
//! истечения. TTL и максимальное число записей настраиваются через конфиг.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Запись кэша: сырой ответ и момент протухания.
struct CacheEntry {
    response: Vec<u8>,
    expires_at: Instant,
}

/// Кэш ответов. Включается/выключается флагом в конфиге.
pub struct Cache {
    enabled: bool,
    max_entries: usize,
    ttl: Duration,
    entries: HashMap<String, CacheEntry>,
}

impl Cache {
    /// Создать кэш. При `enabled = false` все операции — no-op.
    pub fn new(enabled: bool, max_entries: usize, ttl_secs: u64) -> Self {
        Cache {
            enabled,
            max_entries: max_entries.max(1),
            ttl: Duration::from_secs(ttl_secs),
            entries: HashMap::new(),
        }
    }

    /// Сформировать ключ кэша из имени, типа и класса запроса.
    fn key(name: &str, qtype: u16, qclass: u16) -> String {
        format!("{}:{}:{}", name, qtype, qclass)
    }

    /// Достать сырой ответ из кэша, если он есть и не протух.
    ///
    /// Возвращает ответ без подмены DNS-id (это делает вызывающий).
    pub fn get(&self, name: &str, qtype: u16, qclass: u16) -> Option<Vec<u8>> {
        if !self.enabled {
            return None;
        }
        let entry = self.entries.get(&Self::key(name, qtype, qclass))?;
        if entry.expires_at <= Instant::now() {
            // Протухло — игнорируем (физически удалим при следующем put).
            return None;
        }
        Some(entry.response.clone())
    }

    /// Положить сырой ответ в кэш с настроенным TTL.
    ///
    /// Перед вставкой удаляются протухшие записи; при переполнении вытесняются
    /// произвольные записи (без LRU — достаточно для роутера).
    pub fn put(&mut self, name: &str, qtype: u16, qclass: u16, response: Vec<u8>) {
        if !self.enabled {
            return;
        }
        self.cleanup_expired();
        while self.entries.len() >= self.max_entries {
            // Удаляем произвольную запись, чтобы не превысить лимит.
            let Some(key) = self.entries.keys().next().cloned() else {
                break;
            };
            self.entries.remove(&key);
        }
        self.entries.insert(
            Self::key(name, qtype, qclass),
            CacheEntry {
                response,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    /// Удалить все протухшие записи.
    fn cleanup_expired(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, e| e.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_cache_returns_nothing() {
        let mut c = Cache::new(false, 1024, 60);
        c.put("a.lan", 1, 1, vec![1, 2, 3]);
        assert!(c.get("a.lan", 1, 1).is_none());
    }

    #[test]
    fn put_then_get() {
        let mut c = Cache::new(true, 1024, 60);
        c.put("a.lan", 1, 1, vec![1, 2, 3]);
        assert_eq!(c.get("a.lan", 1, 1), Some(vec![1, 2, 3]));
        assert!(c.get("a.lan", 28, 1).is_none()); // другой тип — промах
        assert!(c.get("a.lan", 1, 3).is_none()); // другой класс — промах
    }

    #[test]
    fn expired_entry_ignored() {
        let mut c = Cache::new(true, 1024, 60);
        c.entries.insert(
            Cache::key("old.lan", 1, 1),
            CacheEntry {
                response: vec![9],
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );
        assert!(c.get("old.lan", 1, 1).is_none());
    }

    #[test]
    fn respects_max_entries() {
        let mut c = Cache::new(true, 2, 60);
        c.put("a.lan", 1, 1, vec![1]);
        c.put("b.lan", 1, 1, vec![2]);
        c.put("c.lan", 1, 1, vec![3]);
        // Лимит = 2, поэтому записей не больше двух.
        assert!(c.entries.len() <= 2);
    }
}

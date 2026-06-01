//! Простой кэш DNS-ответов без LRU.
//!
//! Ключ: `name:type`. Значение: сырые байты ответа upstream + момент истечения.
//! TTL в первой версии упрощён до фиксированных 60 секунд.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Фиксированный TTL для записей кэша в первой версии.
const DEFAULT_TTL: Duration = Duration::from_secs(60);

/// Запись кэша: сырой ответ и момент протухания.
struct CacheEntry {
    response: Vec<u8>,
    expires_at: Instant,
}

/// Кэш ответов. Включается/выключается флагом в конфиге.
pub struct Cache {
    enabled: bool,
    entries: HashMap<String, CacheEntry>,
}

impl Cache {
    /// Создать кэш. При `enabled = false` все операции — no-op.
    pub fn new(enabled: bool) -> Self {
        Cache {
            enabled,
            entries: HashMap::new(),
        }
    }

    /// Сформировать ключ кэша из имени и типа запроса.
    fn key(name: &str, qtype: u16) -> String {
        format!("{}:{}", name, qtype)
    }

    /// Достать сырой ответ из кэша, если он есть и не протух.
    ///
    /// Возвращает ответ без DNS-id (его подставляет вызывающий под текущий запрос).
    pub fn get(&self, name: &str, qtype: u16) -> Option<Vec<u8>> {
        if !self.enabled {
            return None;
        }
        let entry = self.entries.get(&Self::key(name, qtype))?;
        if entry.expires_at <= Instant::now() {
            // Протухло — игнорируем (чистка произойдёт при следующем insert).
            return None;
        }
        Some(entry.response.clone())
    }

    /// Положить сырой ответ в кэш с фиксированным TTL.
    pub fn put(&mut self, name: &str, qtype: u16, response: Vec<u8>) {
        if !self.enabled {
            return;
        }
        self.entries.insert(
            Self::key(name, qtype),
            CacheEntry {
                response,
                expires_at: Instant::now() + DEFAULT_TTL,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_cache_returns_nothing() {
        let mut c = Cache::new(false);
        c.put("a.lan", 1, vec![1, 2, 3]);
        assert!(c.get("a.lan", 1).is_none());
    }

    #[test]
    fn put_then_get() {
        let mut c = Cache::new(true);
        c.put("a.lan", 1, vec![1, 2, 3]);
        assert_eq!(c.get("a.lan", 1), Some(vec![1, 2, 3]));
        assert!(c.get("a.lan", 28).is_none()); // другой тип — промах
    }

    #[test]
    fn expired_entry_ignored() {
        let mut c = Cache::new(true);
        // Вставляем вручную уже протухшую запись.
        c.entries.insert(
            Cache::key("old.lan", 1),
            CacheEntry {
                response: vec![9],
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );
        assert!(c.get("old.lan", 1).is_none());
    }
}

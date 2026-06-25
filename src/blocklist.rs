//! Список блокировки доменов (ad/tracking blocking).
//!
//! Формат файла — одна строка = один домен:
//!
//! ```text
//! ads.doubleclick.net
//! tracking.example.com
//! *.adserver.com
//! ```
//!
//! Поддерживается wildcard `*.domain`: блокируется и сам `domain`, и любые его
//! поддомены. Пустые строки и строки с `#` игнорируются.
//!
//! Списки бывают большими (StevenBlack/AdGuard ~150k доменов), поэтому точные
//! совпадения хранятся в `HashSet` (O(1)), а wildcard'ы — как суффиксы.

use std::collections::HashSet;

/// Разобранный список блокировки.
#[derive(Debug, Default)]
pub struct Blocklist {
    /// Точные домены (а также базовые домены wildcard-правил).
    exact: HashSet<String>,
    /// Суффиксы вида ".adserver.com" для wildcard-правил.
    wildcards: Vec<String>,
}

impl Blocklist {
    /// Заблокирован ли домен (имя ожидается в нижнем регистре, без точки в конце).
    pub fn is_blocked(&self, name: &str) -> bool {
        if self.exact.contains(name) {
            return true;
        }
        self.wildcards.iter().any(|suffix| name.ends_with(suffix))
    }

    /// Сколько правил в списке (точные + wildcard).
    pub fn len(&self) -> usize {
        self.exact.len() + self.wildcards.len()
    }

    /// Пуст ли список.
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.wildcards.is_empty()
    }
}

/// Прочитать список из файла. Если файла нет — пустой список (сервер не падает).
pub fn load(path: &str) -> Blocklist {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) => {
            eprintln!(
                "blocklist: не удалось прочитать {}: {} (пропускаю)",
                path, e
            );
            Blocklist::default()
        }
    }
}

/// Разобрать содержимое списка блокировки.
pub fn parse(text: &str) -> Blocklist {
    let mut list = Blocklist::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let domain = line.trim_end_matches('.').to_ascii_lowercase();
        if let Some(base) = domain.strip_prefix("*.") {
            if base.is_empty() {
                continue;
            }
            // Wildcard: блокируем и базовый домен, и все поддомены.
            list.exact.insert(base.to_string());
            list.wildcards.push(format!(".{base}"));
        } else {
            list.exact.insert(domain);
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let list = parse("ads.doubleclick.net\ntracking.example.com\n");
        assert!(list.is_blocked("ads.doubleclick.net"));
        assert!(list.is_blocked("tracking.example.com"));
        assert!(!list.is_blocked("example.com"));
        assert!(!list.is_blocked("doubleclick.net"));
    }

    #[test]
    fn wildcard_match() {
        let list = parse("*.adserver.com\n");
        assert!(list.is_blocked("adserver.com")); // базовый домен
        assert!(list.is_blocked("a.adserver.com")); // поддомен
        assert!(list.is_blocked("x.y.adserver.com")); // вложенный поддомен
        assert!(!list.is_blocked("notadserver.com")); // не суффикс по метке
        assert!(!list.is_blocked("adserver.com.evil.net"));
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let list = parse("# comment\n\n   \nads.test\n");
        assert_eq!(list.len(), 1);
        assert!(list.is_blocked("ads.test"));
    }

    #[test]
    fn case_and_trailing_dot_normalized() {
        let list = parse("Ads.Example.COM.\n");
        assert!(list.is_blocked("ads.example.com"));
    }

    #[test]
    fn missing_file_returns_empty() {
        assert!(load("/nonexistent/blocklist").is_empty());
    }
}

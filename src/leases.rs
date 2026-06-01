//! Чтение DHCP leases.
//!
//! Формат файла (одна аренда на строку):
//!
//! ```text
//! aa:bb:cc:dd:ee:ff 192.168.4.23 phone 1780310000
//! 11:22:33:44:55:66 192.168.4.42 laptop 1780310000
//! ```
//!
//! Поля: MAC, IP, hostname, expiry (unix-время, необязательно).
//! `expiry == 0` трактуется как «бессрочно» (static lease).

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Одна аренда: адрес, имя и срок истечения (unix-секунды; 0 = бессрочно).
#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub ip: Ipv4Addr,
    pub hostname: String,
    pub expires_at: u64,
}

/// Текущее unix-время в секундах.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Прочитать leases из файла в map `hostname -> Lease` (hostname в нижнем регистре).
///
/// Если файла нет — возвращаем пустой map, сервер не должен падать.
pub fn load(path: &str) -> HashMap<String, Lease> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) => {
            eprintln!("leases: не удалось прочитать {}: {} (пропускаю)", path, e);
            HashMap::new()
        }
    }
}

/// Разобрать содержимое lease-файла. Битые и истёкшие строки пропускаются.
pub fn parse(text: &str) -> HashMap<String, Lease> {
    let now = unix_now();
    let mut map = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Нужны хотя бы MAC, IP, hostname.
        if fields.len() < 3 {
            continue;
        }
        let Ok(ip) = fields[1].parse::<Ipv4Addr>() else {
            continue;
        };
        let hostname = fields[2].to_ascii_lowercase();
        if hostname.is_empty() || hostname == "*" {
            continue;
        }
        // expiry необязателен; отсутствие/мусор трактуем как 0 (бессрочно).
        let expires_at = fields
            .get(3)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        // Истёкшие аренды не загружаем.
        if expires_at != 0 && expires_at <= now {
            continue;
        }
        map.insert(
            hostname.clone(),
            Lease {
                ip,
                hostname,
                expires_at,
            },
        );
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_leases() {
        let text = "\
aa:bb:cc:dd:ee:ff 192.168.4.23 phone 0
11:22:33:44:55:66 192.168.4.42 laptop 0
";
        let map = parse(text);
        assert_eq!(map.get("phone").unwrap().ip, Ipv4Addr::new(192, 168, 4, 23));
        assert_eq!(
            map.get("laptop").unwrap().ip,
            Ipv4Addr::new(192, 168, 4, 42)
        );
    }

    #[test]
    fn skip_broken_lines() {
        let text = "\
garbage
aa:bb:cc:dd:ee:ff not-an-ip phone 0
11:22:33:44:55:66 192.168.4.42 laptop 0
";
        let map = parse(text);
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get("laptop").unwrap().ip,
            Ipv4Addr::new(192, 168, 4, 42)
        );
    }

    #[test]
    fn skip_expired_lease() {
        // expires_at в прошлом — аренда не загружается.
        let text = "aa:bb:cc:dd:ee:ff 192.168.4.23 phone 1\n";
        assert!(parse(text).is_empty());
    }

    #[test]
    fn keep_future_and_infinite() {
        let future = unix_now() + 3600;
        let text = format!(
            "aa:bb:cc:dd:ee:ff 192.168.4.23 phone {future}\n\
             11:22:33:44:55:66 192.168.4.42 laptop 0\n"
        );
        let map = parse(&text);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn missing_file_returns_empty() {
        let map = load("/nonexistent/path/leases.txt");
        assert!(map.is_empty());
    }
}

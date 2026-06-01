//! Чтение DHCP leases от nanodhcp.
//!
//! Формат файла (одна аренда на строку):
//!
//! ```text
//! aa:bb:cc:dd:ee:ff 192.168.4.23 phone 1780310000
//! 11:22:33:44:55:66 192.168.4.42 laptop 1780310000
//! ```
//!
//! Поля: MAC, IP, hostname, expiry (unix-время). Для резолва нужны IP и hostname.

use std::collections::HashMap;
use std::net::Ipv4Addr;

/// Прочитать leases из файла в map `hostname -> ip` (hostname в нижнем регистре).
///
/// Если файла нет — возвращаем пустой map, сервер не должен падать.
pub fn load(path: &str) -> HashMap<String, Ipv4Addr> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) => {
            eprintln!("leases: не удалось прочитать {}: {} (пропускаю)", path, e);
            HashMap::new()
        }
    }
}

/// Разобрать содержимое lease-файла. Битые строки пропускаются.
pub fn parse(text: &str) -> HashMap<String, Ipv4Addr> {
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
        map.insert(hostname, ip);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_leases() {
        let text = "\
aa:bb:cc:dd:ee:ff 192.168.4.23 phone 1780310000
11:22:33:44:55:66 192.168.4.42 laptop 1780310000
";
        let map = parse(text);
        assert_eq!(map.get("phone"), Some(&Ipv4Addr::new(192, 168, 4, 23)));
        assert_eq!(map.get("laptop"), Some(&Ipv4Addr::new(192, 168, 4, 42)));
    }

    #[test]
    fn skip_broken_lines() {
        let text = "\
garbage
aa:bb:cc:dd:ee:ff not-an-ip phone 1
11:22:33:44:55:66 192.168.4.42 laptop 1780310000
";
        let map = parse(text);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("laptop"), Some(&Ipv4Addr::new(192, 168, 4, 42)));
    }

    #[test]
    fn missing_file_returns_empty() {
        let map = load("/nonexistent/path/leases.txt");
        assert!(map.is_empty());
    }
}

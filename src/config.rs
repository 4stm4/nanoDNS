//! Чтение конфигурации в формате key=value (без TOML/JSON).
//!
//! Неизвестные ключи и пустые строки игнорируются, строки с `#` — комментарии.

use std::collections::HashMap;
use std::net::Ipv4Addr;

/// Статическая A-запись из конфига.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticRecord {
    pub name: String,
    pub ip: Ipv4Addr,
    pub ttl: u32,
}

/// Полная конфигурация сервера.
#[derive(Debug, Clone)]
pub struct Config {
    pub listen: String,
    pub domain: String,
    pub router_name: String,
    pub router_ip: Ipv4Addr,
    pub upstream: Vec<String>,
    pub lease_file: Option<String>,
    pub cache: bool,
    pub captive: bool,
    pub captive_ip: Ipv4Addr,
    pub records: Vec<StaticRecord>,
}

impl Default for Config {
    /// Дефолты на случай отсутствия файла конфига.
    fn default() -> Self {
        Config {
            listen: "0.0.0.0:5353".to_string(),
            domain: "lan".to_string(),
            router_name: "router".to_string(),
            router_ip: Ipv4Addr::new(192, 168, 4, 1),
            upstream: vec!["1.1.1.1:53".to_string()],
            lease_file: None,
            cache: false,
            captive: false,
            captive_ip: Ipv4Addr::new(192, 168, 4, 1),
            records: Vec::new(),
        }
    }
}

impl Config {
    /// Полное доменное имя роутера, например "router.lan".
    pub fn router_fqdn(&self) -> String {
        format!("{}.{}", self.router_name, self.domain)
    }

    /// Прочитать и разобрать конфиг из файла.
    pub fn from_file(path: &str) -> Result<Config, Box<dyn std::error::Error>> {
        let text = std::fs::read_to_string(path)?;
        Config::parse(&text)
    }

    /// Разобрать конфиг из строки. Поверх дефолтов накладываются заданные ключи.
    pub fn parse(text: &str) -> Result<Config, Box<dyn std::error::Error>> {
        let mut cfg = Config::default();
        // upstream накапливаем сами, поэтому начинаем с пустого списка,
        // если в конфиге встретится хотя бы один upstream.
        let mut upstream: Vec<String> = Vec::new();
        let mut records: Vec<StaticRecord> = Vec::new();
        // Простые ключи складываем в map, чтобы последнее значение побеждало.
        let mut simple: HashMap<String, String> = HashMap::new();

        for (lineno, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                eprintln!(
                    "config: строка {}: нет '=', пропускаю: {}",
                    lineno + 1,
                    line
                );
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "upstream" => upstream.push(value.to_string()),
                "record" => match parse_record(value) {
                    Ok(rec) => records.push(rec),
                    Err(e) => eprintln!(
                        "config: строка {}: плохая record '{}': {}",
                        lineno + 1,
                        value,
                        e
                    ),
                },
                _ => {
                    simple.insert(key.to_string(), value.to_string());
                }
            }
        }

        if let Some(v) = simple.get("listen") {
            cfg.listen = v.clone();
        }
        if let Some(v) = simple.get("domain") {
            cfg.domain = v.to_ascii_lowercase();
        }
        if let Some(v) = simple.get("router_name") {
            cfg.router_name = v.to_ascii_lowercase();
        }
        if let Some(v) = simple.get("router_ip") {
            cfg.router_ip = v.parse()?;
        }
        if let Some(v) = simple.get("lease_file") {
            cfg.lease_file = Some(v.clone());
        }
        if let Some(v) = simple.get("cache") {
            cfg.cache = parse_bool(v);
        }
        if let Some(v) = simple.get("captive") {
            cfg.captive = parse_bool(v);
        }
        if let Some(v) = simple.get("captive_ip") {
            cfg.captive_ip = v.parse()?;
        }
        if !upstream.is_empty() {
            cfg.upstream = upstream;
        }
        cfg.records = records;

        Ok(cfg)
    }
}

/// Разобрать "true"/"1"/"yes" как true (без учёта регистра).
fn parse_bool(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
}

/// Разобрать одну запись формата `name,A,ip,ttl`.
fn parse_record(value: &str) -> Result<StaticRecord, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
    if parts.len() != 4 {
        return Err("ожидается формат name,A,ip,ttl".into());
    }
    if !parts[1].eq_ignore_ascii_case("A") {
        return Err("поддерживается только тип A".into());
    }
    let ip: Ipv4Addr = parts[2].parse()?;
    let ttl: u32 = parts[3].parse()?;
    Ok(StaticRecord {
        name: parts[0].to_ascii_lowercase(),
        ip,
        ttl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let cfg = Config::parse("").unwrap();
        assert_eq!(cfg.listen, "0.0.0.0:5353");
        assert_eq!(cfg.domain, "lan");
        assert_eq!(cfg.upstream, vec!["1.1.1.1:53".to_string()]);
        assert!(!cfg.cache);
    }

    #[test]
    fn parse_full_config() {
        let text = "\
listen=0.0.0.0:5353
domain=lan
router_name=router
router_ip=192.168.4.1
upstream=1.1.1.1:53
upstream=8.8.8.8:53
lease_file=/var/lib/nanodhcp/leases.txt
cache=true
captive=false
captive_ip=192.168.4.1
record=admin.lan,A,192.168.4.1,60
record=router.lan,A,192.168.4.1,60
";
        let cfg = Config::parse(text).unwrap();
        assert_eq!(cfg.domain, "lan");
        assert_eq!(cfg.router_fqdn(), "router.lan");
        assert_eq!(cfg.upstream, vec!["1.1.1.1:53", "8.8.8.8:53"]);
        assert_eq!(
            cfg.lease_file.as_deref(),
            Some("/var/lib/nanodhcp/leases.txt")
        );
        assert!(cfg.cache);
        assert!(!cfg.captive);
        assert_eq!(cfg.records.len(), 2);
        assert_eq!(
            cfg.records[0],
            StaticRecord {
                name: "admin.lan".to_string(),
                ip: Ipv4Addr::new(192, 168, 4, 1),
                ttl: 60,
            }
        );
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let text = "# комментарий\n\n   \ndomain=home\n";
        let cfg = Config::parse(text).unwrap();
        assert_eq!(cfg.domain, "home");
    }

    #[test]
    fn bad_record_does_not_fail_parse() {
        // Плохая запись логируется и пропускается, конфиг остаётся валидным.
        let text = "record=broken\nrecord=ok.lan,A,10.0.0.1,30\n";
        let cfg = Config::parse(text).unwrap();
        assert_eq!(cfg.records.len(), 1);
        assert_eq!(cfg.records[0].name, "ok.lan");
    }
}

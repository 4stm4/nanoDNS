//! Resolver: принимает разобранный запрос и решает, как на него ответить.
//!
//! Порядок обработки (строго):
//!   1. captive mode (если включён)
//!   2. router_name.domain
//!   3. статические записи из конфига
//!   4. leases из lease_file
//!   5. cache (если включён)
//!   6. форвард на upstream
//!   7. SERVFAIL / NXDOMAIN
//!
//! Resolver сознательно отделён от парсинга пакетов (модуль `dns`).

use std::collections::HashMap;
use std::net::Ipv4Addr;

use crate::cache::Cache;
use crate::config::Config;
use crate::dns::{self, Query};
use crate::forward;

/// Состояние резолвера: конфиг, leases и кэш.
pub struct Resolver {
    config: Config,
    leases: HashMap<String, Ipv4Addr>,
    cache: Cache,
    router_fqdn: String,
}

impl Resolver {
    /// Собрать резолвер. Leases читаются здесь же (отсутствие файла не фатально).
    pub fn new(config: Config) -> Self {
        let leases = match &config.lease_file {
            Some(path) => crate::leases::load(path),
            None => HashMap::new(),
        };
        let cache = Cache::new(config.cache);
        let router_fqdn = config.router_fqdn();
        Resolver {
            config,
            leases,
            cache,
            router_fqdn,
        }
    }

    /// Является ли имя частью локальной зоны (domain или *.domain).
    fn is_local_zone(&self, name: &str) -> bool {
        let domain = &self.config.domain;
        name == domain || name.ends_with(&format!(".{}", domain))
    }

    /// Найти локальную A-запись (router / config / leases) без учёта кэша.
    fn local_a(&self, name: &str) -> Option<(Ipv4Addr, u32)> {
        // 2. router
        if name == self.router_fqdn {
            return Some((self.config.router_ip, 60));
        }
        // 3. статические записи
        for rec in &self.config.records {
            if rec.name == name {
                return Some((rec.ip, rec.ttl));
            }
        }
        // 4. leases: hostname = имя без суффикса ".domain"
        let suffix = format!(".{}", self.config.domain);
        if let Some(host) = name.strip_suffix(&suffix) {
            if let Some(ip) = self.leases.get(host) {
                return Some((*ip, 60));
            }
        }
        None
    }

    /// Разрешить запрос и вернуть сырые байты ответа клиенту.
    ///
    /// `raw` — оригинальный пакет (нужен для форвардинга без потерь).
    pub fn resolve(&mut self, query: &Query, raw: &[u8]) -> Vec<u8> {
        let name = query.question.name.clone();
        let qtype = query.question.qtype;
        let qclass = query.question.qclass;
        let is_a_in = qtype == dns::TYPE_A && qclass == dns::CLASS_IN;

        // 1. captive mode: любой A/IN-запрос получает captive_ip.
        // Не-A/не-IN в captive форвардим наружу (зафиксировано в README).
        if self.config.captive {
            if is_a_in {
                println!("resolve: captive {} -> {}", name, self.config.captive_ip);
                return dns::build_a_response(query, self.config.captive_ip, 60);
            }
            return self.forward_or_servfail(query, raw);
        }

        // 2-4. локальные A-записи.
        if is_a_in {
            if let Some((ip, ttl)) = self.local_a(&name) {
                println!("resolve: local {} -> {}", name, ip);
                return dns::build_a_response(query, ip, ttl);
            }
        }

        // 5. cache.
        if let Some(mut cached) = self.cache.get(&name, qtype) {
            // Подменяем DNS-id под текущий запрос.
            let id = query.header.id.to_be_bytes();
            if cached.len() >= 2 {
                cached[0] = id[0];
                cached[1] = id[1];
            }
            println!("resolve: cache hit {}", name);
            return cached;
        }

        // Локальная зона без записи — NXDOMAIN, наружу не ходим.
        if is_a_in && self.is_local_zone(&name) {
            println!("resolve: NXDOMAIN {} (локальная зона, записи нет)", name);
            return dns::build_error_response(query, dns::RCODE_NXDOMAIN);
        }

        // 6-7. форвард на upstream, иначе SERVFAIL.
        self.forward_or_servfail(query, raw)
    }

    /// Переслать запрос наружу; при успехе кэшировать, при неудаче — SERVFAIL.
    fn forward_or_servfail(&mut self, query: &Query, raw: &[u8]) -> Vec<u8> {
        match forward::forward(raw, &self.config.upstream) {
            Ok(resp) => {
                self.cache
                    .put(&query.question.name, query.question.qtype, resp.clone());
                resp
            }
            Err(e) => {
                eprintln!("resolve: SERVFAIL {}: {}", query.question.name, e);
                dns::build_error_response(query, dns::RCODE_SERVFAIL)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StaticRecord;
    use crate::dns::{CLASS_IN, TYPE_A};

    /// Собрать query-структуру напрямую для тестов резолва.
    fn query(name: &str, qtype: u16) -> Query {
        Query {
            header: dns::Header {
                id: 1,
                flags: 0x0100,
                qdcount: 1,
                ancount: 0,
                nscount: 0,
                arcount: 0,
            },
            question: dns::Question {
                name: name.to_string(),
                qtype,
                qclass: CLASS_IN,
            },
        }
    }

    /// Достать IPv4 из хвоста A-ответа.
    fn answer_ip(resp: &[u8]) -> Ipv4Addr {
        let n = resp.len();
        Ipv4Addr::new(resp[n - 4], resp[n - 3], resp[n - 2], resp[n - 1])
    }

    fn rcode(resp: &[u8]) -> u8 {
        (u16::from_be_bytes([resp[2], resp[3]]) & 0x000F) as u8
    }

    fn base_config() -> Config {
        let mut cfg = Config::default();
        cfg.domain = "lan".to_string();
        cfg.router_name = "router".to_string();
        cfg.router_ip = Ipv4Addr::new(192, 168, 4, 1);
        cfg.records = vec![StaticRecord {
            name: "admin.lan".to_string(),
            ip: Ipv4Addr::new(192, 168, 4, 5),
            ttl: 60,
        }];
        cfg
    }

    #[test]
    fn resolve_router() {
        let mut r = Resolver::new(base_config());
        let resp = r.resolve(&query("router.lan", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(192, 168, 4, 1));
    }

    #[test]
    fn resolve_static_record() {
        let mut r = Resolver::new(base_config());
        let resp = r.resolve(&query("admin.lan", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(192, 168, 4, 5));
    }

    #[test]
    fn resolve_lease_hostname() {
        let mut r = Resolver::new(base_config());
        r.leases
            .insert("phone".to_string(), Ipv4Addr::new(192, 168, 4, 23));
        let resp = r.resolve(&query("phone.lan", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(192, 168, 4, 23));
    }

    #[test]
    fn unknown_local_zone_is_nxdomain() {
        let mut r = Resolver::new(base_config());
        let resp = r.resolve(&query("ghost.lan", TYPE_A), &[]);
        assert_eq!(rcode(&resp), dns::RCODE_NXDOMAIN);
    }

    #[test]
    fn captive_returns_captive_ip() {
        let mut cfg = base_config();
        cfg.captive = true;
        cfg.captive_ip = Ipv4Addr::new(10, 0, 0, 1);
        let mut r = Resolver::new(cfg);
        // Даже для произвольного внешнего домена в captive — captive_ip.
        let resp = r.resolve(&query("example.com", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(10, 0, 0, 1));
    }
}

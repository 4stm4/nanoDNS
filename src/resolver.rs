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
//!
//! Все методы берут `&self` (внутренняя изменяемость через `Mutex`), чтобы
//! резолвер можно было шарить между потоками в `Arc`. Форвард на upstream
//! выполняется БЕЗ удержания блокировок, чтобы медленный upstream не блокировал
//! локальный резолвинг других запросов.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use crate::blocklist::{self, Blocklist};
use crate::cache::Cache;
use crate::config::{BlockResponse, Config};
use crate::dns::{self, Query};
use crate::forward;
use crate::leases::{self, Lease};

/// Как часто разрешено проверять mtime файлов (не чаще, чем раз в N).
const RELOAD_INTERVAL: Duration = Duration::from_secs(2);

/// Хранилище leases с метаданными для перезагрузки по mtime.
struct LeaseStore {
    path: Option<String>,
    table: HashMap<String, Lease>,
    last_reload: Instant,
    last_mtime: Option<SystemTime>,
}

impl LeaseStore {
    /// Перечитать leases, если прошёл интервал и mtime файла изменился.
    fn maybe_reload(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.last_reload.elapsed() < RELOAD_INTERVAL {
            return;
        }
        self.last_reload = Instant::now();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if mtime != self.last_mtime {
            println!("leases: файл изменился, перезагружаю {}", path);
            self.table = leases::load(&path);
            self.last_mtime = mtime;
        }
    }
}

/// Хранилище blocklist с метаданными для перезагрузки по mtime.
struct BlockStore {
    path: Option<String>,
    list: Blocklist,
    last_reload: Instant,
    last_mtime: Option<SystemTime>,
}

impl BlockStore {
    /// Перечитать blocklist, если прошёл интервал и mtime файла изменился.
    fn maybe_reload(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.last_reload.elapsed() < RELOAD_INTERVAL {
            return;
        }
        self.last_reload = Instant::now();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if mtime != self.last_mtime {
            self.list = blocklist::load(&path);
            self.last_mtime = mtime;
            println!(
                "blocklist: файл изменился, перезагрузил {} ({} правил)",
                path,
                self.list.len()
            );
        }
    }
}

/// Состояние резолвера: конфиг, leases, blocklist и кэш.
pub struct Resolver {
    config: Config,
    leases: Mutex<LeaseStore>,
    blocklist: Mutex<BlockStore>,
    cache: Mutex<Cache>,
    router_fqdn: String,
}

impl Resolver {
    /// Собрать резолвер. Leases читаются здесь же (отсутствие файла не фатально).
    pub fn new(config: Config) -> Self {
        let path = config.lease_file.clone();
        let table = match &path {
            Some(p) => leases::load(p),
            None => HashMap::new(),
        };
        let last_mtime = path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        let leases = Mutex::new(LeaseStore {
            path,
            table,
            last_reload: Instant::now(),
            last_mtime,
        });

        let block_path = config.block_file.clone();
        let list = match &block_path {
            Some(p) => blocklist::load(p),
            None => Blocklist::default(),
        };
        if !list.is_empty() {
            println!("blocklist: загружено {} правил", list.len());
        }
        let block_mtime = block_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        let blocklist = Mutex::new(BlockStore {
            path: block_path,
            list,
            last_reload: Instant::now(),
            last_mtime: block_mtime,
        });

        let cache = Mutex::new(Cache::new(
            config.cache,
            config.cache_max_entries,
            config.cache_ttl,
        ));
        let router_fqdn = config.router_fqdn();
        Resolver {
            config,
            leases,
            blocklist,
            cache,
            router_fqdn,
        }
    }

    /// Заблокирован ли домен (с учётом возможной перезагрузки файла).
    fn is_blocked(&self, name: &str) -> bool {
        let mut store = self.blocklist.lock().unwrap();
        store.maybe_reload();
        store.list.is_blocked(name)
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
            let mut store = self.leases.lock().unwrap();
            store.maybe_reload();
            if let Some(lease) = store.table.get(host) {
                let now = leases::unix_now();
                // Истёкшие аренды не резолвим (0 = бессрочно).
                if lease.expires_at == 0 || lease.expires_at > now {
                    return Some((lease.ip, 60));
                }
            }
        }
        None
    }

    /// Разрешить запрос и вернуть сырые байты ответа клиенту.
    ///
    /// `raw` — оригинальный пакет (нужен для форвардинга без потерь).
    pub fn resolve(&self, query: &Query, raw: &[u8]) -> Vec<u8> {
        let name = query.question.name.clone();
        let qtype = query.question.qtype;
        let qclass = query.question.qclass;
        let is_a_in = qtype == dns::TYPE_A && qclass == dns::CLASS_IN;

        // 1. captive mode: любой A/IN-запрос получает captive_ip.
        // Не-A/не-IN в captive форвардим наружу (зафиксировано в README).
        if self.config.captive {
            if is_a_in {
                println!("resolve: captive {} -> {}", name, self.config.captive_ip);
                return dns::build_a_response(query, self.config.captive_ip, 60, true);
            }
            return self.forward_or_servfail(query, raw);
        }

        // 2-4. локальные A-записи (authoritative). Локальные имена приоритетнее
        // блок-листа, чтобы случайно не заблокировать собственные сервисы.
        if is_a_in {
            if let Some((ip, ttl)) = self.local_a(&name) {
                println!("resolve: local {} -> {}", name, ip);
                return dns::build_a_response(query, ip, ttl, true);
            }
        }

        // Блок-лист: заблокированный домен не форвардим и не кэшируем.
        if self.is_blocked(&name) {
            println!("resolve: blocked {}", name);
            return match self.config.block_response {
                BlockResponse::NxDomain => {
                    dns::build_error_response(query, dns::RCODE_NXDOMAIN, true)
                }
                BlockResponse::Ip(ip) if is_a_in => dns::build_a_response(query, ip, 60, true),
                // Не-A запрос к заблокированному домену — отдаём NXDOMAIN,
                // чтобы он не ушёл наружу.
                BlockResponse::Ip(_) => dns::build_error_response(query, dns::RCODE_NXDOMAIN, true),
            };
        }

        // 5. cache.
        if let Some(mut cached) = self.cache.lock().unwrap().get(&name, qtype, qclass) {
            // Подменяем DNS-id под текущий запрос.
            let id = query.header.id.to_be_bytes();
            if cached.len() >= 2 {
                cached[0] = id[0];
                cached[1] = id[1];
            }
            println!("resolve: cache hit {}", name);
            return cached;
        }

        // Локальная зона без записи — NXDOMAIN (authoritative), наружу не ходим.
        if is_a_in && self.is_local_zone(&name) {
            println!("resolve: NXDOMAIN {} (локальная зона, записи нет)", name);
            return dns::build_error_response(query, dns::RCODE_NXDOMAIN, true);
        }

        // 6-7. форвард на upstream, иначе SERVFAIL.
        self.forward_or_servfail(query, raw)
    }

    /// Переслать запрос наружу; при успехе кэшировать, при неудаче — SERVFAIL.
    ///
    /// Сетевой форвард выполняется без удержания блокировок; кэш лочим только
    /// на короткую вставку результата.
    fn forward_or_servfail(&self, query: &Query, raw: &[u8]) -> Vec<u8> {
        match forward::forward(raw, &self.config.upstream) {
            Ok(resp) => {
                self.cache.lock().unwrap().put(
                    &query.question.name,
                    query.question.qtype,
                    query.question.qclass,
                    resp.clone(),
                );
                resp
            }
            Err(e) => {
                eprintln!("resolve: SERVFAIL {}: {}", query.question.name, e);
                // SERVFAIL не authoritative.
                dns::build_error_response(query, dns::RCODE_SERVFAIL, false)
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

    /// Вставить аренду в leases-таблицу резолвера (для тестов).
    fn insert_lease(r: &Resolver, host: &str, ip: Ipv4Addr, expires_at: u64) {
        r.leases.lock().unwrap().table.insert(
            host.to_string(),
            Lease {
                ip,
                hostname: host.to_string(),
                expires_at,
            },
        );
    }

    /// Заменить blocklist резолвера разобранным из текста (для тестов).
    fn set_blocklist(r: &Resolver, text: &str) {
        r.blocklist.lock().unwrap().list = crate::blocklist::parse(text);
    }

    #[test]
    fn resolve_router() {
        let r = Resolver::new(base_config());
        let resp = r.resolve(&query("router.lan", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(192, 168, 4, 1));
        // Локальный ответ — authoritative.
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        assert_eq!(flags & 0x0400, 0x0400);
    }

    #[test]
    fn resolve_static_record() {
        let r = Resolver::new(base_config());
        let resp = r.resolve(&query("admin.lan", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(192, 168, 4, 5));
    }

    #[test]
    fn resolve_lease_hostname() {
        let r = Resolver::new(base_config());
        insert_lease(&r, "phone", Ipv4Addr::new(192, 168, 4, 23), 0);
        let resp = r.resolve(&query("phone.lan", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(192, 168, 4, 23));
    }

    #[test]
    fn expired_lease_is_nxdomain() {
        let r = Resolver::new(base_config());
        // expires_at = 1 (в прошлом) — аренда не должна резолвиться.
        insert_lease(&r, "ghostphone", Ipv4Addr::new(192, 168, 4, 99), 1);
        let resp = r.resolve(&query("ghostphone.lan", TYPE_A), &[]);
        assert_eq!(rcode(&resp), dns::RCODE_NXDOMAIN);
    }

    #[test]
    fn unknown_local_zone_is_nxdomain() {
        let r = Resolver::new(base_config());
        let resp = r.resolve(&query("ghost.lan", TYPE_A), &[]);
        assert_eq!(rcode(&resp), dns::RCODE_NXDOMAIN);
    }

    #[test]
    fn blocked_domain_returns_sinkhole_ip() {
        let r = Resolver::new(base_config()); // block_response по умолчанию = 0.0.0.0
        set_blocklist(&r, "ads.doubleclick.net\n");
        let resp = r.resolve(&query("ads.doubleclick.net", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::UNSPECIFIED);
    }

    #[test]
    fn blocked_domain_can_return_nxdomain() {
        let mut cfg = base_config();
        cfg.block_response = crate::config::BlockResponse::NxDomain;
        let r = Resolver::new(cfg);
        set_blocklist(&r, "*.adserver.com\n");
        let resp = r.resolve(&query("track.adserver.com", TYPE_A), &[]);
        assert_eq!(rcode(&resp), dns::RCODE_NXDOMAIN);
    }

    #[test]
    fn local_record_beats_blocklist() {
        let r = Resolver::new(base_config());
        // admin.lan есть в static records и одновременно в блок-листе.
        set_blocklist(&r, "admin.lan\n");
        let resp = r.resolve(&query("admin.lan", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(192, 168, 4, 5));
    }

    #[test]
    fn captive_returns_captive_ip() {
        let mut cfg = base_config();
        cfg.captive = true;
        cfg.captive_ip = Ipv4Addr::new(10, 0, 0, 1);
        let r = Resolver::new(cfg);
        // Даже для произвольного внешнего домена в captive — captive_ip.
        let resp = r.resolve(&query("example.com", TYPE_A), &[]);
        assert_eq!(answer_ip(&resp), Ipv4Addr::new(10, 0, 0, 1));
    }
}

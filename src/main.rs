//! nanodns — минимальный DNS-сервер для локальной Wi-Fi сети TinyWifi.
//!
//! Запуск:
//!   nanodns --config ./config.example
//!
//! Если --config не передан, используется /etc/nanodns/config.
//! Если файла нет — применяются встроенные дефолты.

mod blocklist;
mod cache;
mod config;
mod dns;
mod forward;
mod leases;
mod resolver;
mod server;

use config::{BlockResponse, Config};

/// Путь к конфигу по умолчанию.
const DEFAULT_CONFIG_PATH: &str = "/etc/nanodns/config";

fn main() {
    let config_path = parse_args(std::env::args().skip(1));

    let config = load_config(&config_path);

    // Стартовый лог.
    println!("nanodns: старт");
    println!("nanodns: конфиг: {}", config_path);
    println!("nanodns: listen: {}", config.listen);
    println!("nanodns: локальная зона: {}", config.domain);
    println!("nanodns: upstream: {}", config.upstream.join(", "));
    if config.cache {
        println!(
            "nanodns: cache: вкл (max_entries={}, ttl={}s)",
            config.cache_max_entries, config.cache_ttl
        );
    } else {
        println!("nanodns: cache: выкл");
    }
    println!("nanodns: max_inflight: {}", config.max_inflight);
    if let Some(path) = &config.block_file {
        let what = match config.block_response {
            BlockResponse::NxDomain => "NXDOMAIN".to_string(),
            BlockResponse::Ip(ip) => ip.to_string(),
        };
        println!("nanodns: blocklist: {} -> {}", path, what);
    }
    if config.captive {
        println!("nanodns: captive mode ВКЛЮЧЁН -> {}", config.captive_ip);
    }

    if let Err(e) = server::run(config) {
        eprintln!("nanodns: фатальная ошибка сервера: {}", e);
        std::process::exit(1);
    }
}

/// Разобрать аргументы и вернуть путь к конфигу.
fn parse_args(args: impl Iterator<Item = String>) -> String {
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                if let Some(path) = args.next() {
                    return path;
                }
                eprintln!("nanodns: --config без значения, использую дефолт");
            }
            "-h" | "--help" => {
                println!("Использование: nanodns [--config <path>]");
                std::process::exit(0);
            }
            other => {
                eprintln!("nanodns: неизвестный аргумент: {}", other);
            }
        }
    }
    DEFAULT_CONFIG_PATH.to_string()
}

/// Загрузить конфиг из файла; при ошибке вернуть дефолты.
fn load_config(path: &str) -> Config {
    match Config::from_file(path) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "nanodns: не удалось прочитать конфиг {} ({}), использую дефолты",
                path, e
            );
            Config::default()
        }
    }
}

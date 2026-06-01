//! UDP DNS-сервер: цикл приёма пакетов и отправки ответов.
//!
//! Сервер однопоточный и устойчив к плохим пакетам: ошибка парсинга
//! логируется, и цикл продолжается.

use std::net::UdpSocket;

use crate::config::Config;
use crate::dns;
use crate::resolver::Resolver;

/// Максимальный размер UDP DNS-пакета, который мы принимаем.
const MAX_PACKET: usize = 4096;

/// Запустить сервер: открыть сокет и обслуживать запросы в бесконечном цикле.
pub fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let listen = config.listen.clone();
    let socket = UdpSocket::bind(&listen)?;
    println!("nanodns: слушаю UDP на {}", listen);

    let mut resolver = Resolver::new(config);
    let mut buf = [0u8; MAX_PACKET];

    loop {
        // Ошибку recv логируем, но не падаем — продолжаем обслуживать.
        let (n, peer) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("server: ошибка recv_from: {}", e);
                continue;
            }
        };
        let raw = &buf[..n];

        let query = match dns::parse_query(raw) {
            Ok(q) => q,
            Err(e) => {
                eprintln!("server: не разобрал пакет от {}: {}", peer, e);
                continue;
            }
        };

        // Отвечаем только на запросы (QR=0) со стандартным opcode.
        if !query.header.is_query() || query.header.opcode() != 0 {
            eprintln!("server: пропускаю не-запрос от {}", peer);
            continue;
        }

        let response = resolver.resolve(&query, raw);

        if let Err(e) = socket.send_to(&response, peer) {
            eprintln!("server: не смог отправить ответ {}: {}", peer, e);
        }
    }
}

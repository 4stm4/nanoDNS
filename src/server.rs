//! UDP DNS-сервер: цикл приёма пакетов и отправки ответов.
//!
//! Каждый запрос обрабатывается в отдельном потоке (thread-per-request), чтобы
//! медленный upstream не блокировал обслуживание других клиентов. Число
//! одновременно работающих потоков ограничено `max_inflight`; при достижении
//! лимита запрос обрабатывается синхронно в основном цикле.
//!
//! Сервер устойчив к плохим пакетам: ошибка парсинга логируется, и обработка
//! продолжается.

use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use crate::config::Config;
use crate::dns;
use crate::resolver::Resolver;

/// Максимальный размер UDP DNS-пакета, который мы принимаем.
const MAX_PACKET: usize = 4096;

/// Запустить сервер: открыть сокет и обслуживать запросы в бесконечном цикле.
pub fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let listen = config.listen.clone();
    let max_inflight = config.max_inflight.max(1);
    let socket = Arc::new(UdpSocket::bind(&listen)?);
    println!("nanodns: слушаю UDP на {}", listen);

    let resolver = Arc::new(Resolver::new(config));
    let inflight = Arc::new(AtomicUsize::new(0));
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
        let raw = buf[..n].to_vec();

        if inflight.load(Ordering::Relaxed) >= max_inflight {
            // Достигнут лимит потоков — обрабатываем синхронно.
            handle_request(&resolver, &socket, &raw, peer);
        } else {
            inflight.fetch_add(1, Ordering::Relaxed);
            let resolver = Arc::clone(&resolver);
            let socket = Arc::clone(&socket);
            let inflight = Arc::clone(&inflight);
            thread::spawn(move || {
                handle_request(&resolver, &socket, &raw, peer);
                inflight.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }
}

/// Разобрать один запрос, срезолвить и отправить ответ клиенту.
fn handle_request(resolver: &Resolver, socket: &UdpSocket, raw: &[u8], peer: std::net::SocketAddr) {
    let query = match dns::parse_query(raw) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("server: не разобрал пакет от {}: {}", peer, e);
            return;
        }
    };

    // Отвечаем только на запросы (QR=0) со стандартным opcode.
    if !query.header.is_query() || query.header.opcode() != 0 {
        eprintln!("server: пропускаю не-запрос от {}", peer);
        return;
    }

    let response = resolver.resolve(&query, raw);

    if let Err(e) = socket.send_to(&response, peer) {
        eprintln!("server: не смог отправить ответ {}: {}", peer, e);
    }
}

//! Форвардинг DNS-запросов на upstream-серверы по UDP.
//!
//! Оригинальный пакет отправляется на upstream как есть, ответ ждём с таймаутом.
//! Серверы перебираются по очереди; если ни один не ответил — ошибка.

use std::net::UdpSocket;
use std::time::Duration;

/// Таймаут ожидания ответа от одного upstream.
const UPSTREAM_TIMEOUT: Duration = Duration::from_millis(1200);

/// Переслать `packet` на upstream-серверы по очереди и вернуть первый ответ.
///
/// `upstreams` — список адресов вида "1.1.1.1:53". Возвращает сырые байты ответа.
pub fn forward(packet: &[u8], upstreams: &[String]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if upstreams.is_empty() {
        return Err("не задано ни одного upstream".into());
    }

    let mut last_err: Option<String> = None;

    for addr in upstreams {
        match query_one(packet, addr) {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                eprintln!("forward: upstream {} не ответил: {}", addr, e);
                last_err = Some(e.to_string());
            }
        }
    }

    Err(format!(
        "все upstream недоступны (последняя ошибка: {})",
        last_err.unwrap_or_else(|| "нет".to_string())
    )
    .into())
}

/// Отправить пакет на один upstream и дождаться ответа с таймаутом.
fn query_one(packet: &[u8], addr: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Эфемерный сокет на каждый запрос — просто и потокобезопасно.
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
    sock.set_write_timeout(Some(UPSTREAM_TIMEOUT))?;
    sock.connect(addr)?;
    sock.send(packet)?;

    let mut buf = [0u8; 4096];
    let n = sock.recv(&mut buf)?;
    Ok(buf[..n].to_vec())
}

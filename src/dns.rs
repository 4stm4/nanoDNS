//! Парсинг и сборка DNS-пакетов.
//!
//! Реализован минимум, нужный для MVP: заголовок, один question,
//! доменное имя без compression, A-записи (QTYPE=1, QCLASS=IN).
//!
//! Парсинг и резолвинг намеренно разделены: этот модуль ничего не знает
//! о конфиге, leases или upstream — только о байтах на проводе.

use std::net::Ipv4Addr;

/// QTYPE A — адресная запись IPv4.
pub const TYPE_A: u16 = 1;
/// QCLASS IN — Internet.
pub const CLASS_IN: u16 = 1;

// Коды ответа (RCODE) из заголовка DNS. Полный набор оставлен для читаемости
// и будущего использования (captive non-A и т.п.), часть пока не задействована.
#[allow(dead_code)]
pub const RCODE_NOERROR: u8 = 0;
#[allow(dead_code)]
pub const RCODE_FORMERR: u8 = 1;
pub const RCODE_SERVFAIL: u8 = 2;
pub const RCODE_NXDOMAIN: u8 = 3;
#[allow(dead_code)]
pub const RCODE_NOTIMP: u8 = 4;

/// Ошибки парсинга DNS-пакета. Они не фатальны для сервера —
/// один плохой пакет логируется и пропускается.
#[derive(Debug)]
pub enum DnsError {
    /// Пакет короче 12 байт — нет даже заголовка.
    TooShort,
    /// Неожиданный конец данных при чтении.
    UnexpectedEof,
    /// Имя слишком длинное или зациклено.
    BadName,
    /// В пакете нет ни одного question.
    NoQuestion,
}

impl std::fmt::Display for DnsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            DnsError::TooShort => "пакет короче заголовка DNS",
            DnsError::UnexpectedEof => "неожиданный конец пакета",
            DnsError::BadName => "некорректное доменное имя",
            DnsError::NoQuestion => "в пакете нет question",
        };
        f.write_str(s)
    }
}

impl std::error::Error for DnsError {}

/// Заголовок DNS-пакета (12 байт). Все поля парсятся для полноты,
/// даже если в MVP читаются не все счётчики.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Header {
    pub id: u16,
    pub flags: u16,
    pub qdcount: u16,
    pub ancount: u16,
    pub nscount: u16,
    pub arcount: u16,
}

impl Header {
    /// Признак запроса (QR=0) против ответа (QR=1).
    pub fn is_query(&self) -> bool {
        self.flags & 0x8000 == 0
    }

    /// OPCODE из флагов (0 = стандартный запрос).
    pub fn opcode(&self) -> u8 {
        ((self.flags >> 11) & 0x0F) as u8
    }
}

/// Question-секция: имя + тип + класс.
#[derive(Debug, Clone)]
pub struct Question {
    /// Имя в нижнем регистре с точками между метками, например "router.lan".
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

/// Разобранный запрос: заголовок + первый question.
#[derive(Debug, Clone)]
pub struct Query {
    pub header: Header,
    pub question: Question,
}

/// Прочитать заголовок DNS из начала буфера.
pub fn parse_header(buf: &[u8]) -> Result<Header, DnsError> {
    if buf.len() < 12 {
        return Err(DnsError::TooShort);
    }
    Ok(Header {
        id: u16::from_be_bytes([buf[0], buf[1]]),
        flags: u16::from_be_bytes([buf[2], buf[3]]),
        qdcount: u16::from_be_bytes([buf[4], buf[5]]),
        ancount: u16::from_be_bytes([buf[6], buf[7]]),
        nscount: u16::from_be_bytes([buf[8], buf[9]]),
        arcount: u16::from_be_bytes([buf[10], buf[11]]),
    })
}

/// Прочитать доменное имя из позиции `pos` без поддержки compression.
///
/// Возвращает имя в нижнем регистре и позицию сразу за именем.
pub fn parse_name(buf: &[u8], mut pos: usize) -> Result<(String, usize), DnsError> {
    let mut labels: Vec<String> = Vec::new();
    let mut total = 0usize;
    loop {
        let len = *buf.get(pos).ok_or(DnsError::UnexpectedEof)? as usize;
        pos += 1;
        if len == 0 {
            break;
        }
        // Compression-указатель (старшие два бита) в question мы не поддерживаем.
        if len & 0xC0 != 0 {
            return Err(DnsError::BadName);
        }
        // RFC 1035: длина одной метки не больше 63 октетов.
        if len > 63 {
            return Err(DnsError::BadName);
        }
        // RFC 1035: всё имя не длиннее 255 октетов.
        total += len + 1;
        if total > 255 {
            return Err(DnsError::BadName);
        }
        let end = pos + len;
        let raw = buf.get(pos..end).ok_or(DnsError::UnexpectedEof)?;
        let label = std::str::from_utf8(raw)
            .map_err(|_| DnsError::BadName)?
            .to_ascii_lowercase();
        if !is_valid_label(&label) {
            return Err(DnsError::BadName);
        }
        labels.push(label);
        pos = end;
    }
    Ok((labels.join("."), pos))
}

/// Проверка одной метки hostname: только `a-z 0-9 - _`, без дефиса по краям.
fn is_valid_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
        return false;
    }
    label
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Разобрать запрос целиком: заголовок и первый question.
pub fn parse_query(buf: &[u8]) -> Result<Query, DnsError> {
    let header = parse_header(buf)?;
    if header.qdcount == 0 {
        return Err(DnsError::NoQuestion);
    }
    let (name, pos) = parse_name(buf, 12)?;
    let qtype = u16::from_be_bytes([
        *buf.get(pos).ok_or(DnsError::UnexpectedEof)?,
        *buf.get(pos + 1).ok_or(DnsError::UnexpectedEof)?,
    ]);
    let qclass = u16::from_be_bytes([
        *buf.get(pos + 2).ok_or(DnsError::UnexpectedEof)?,
        *buf.get(pos + 3).ok_or(DnsError::UnexpectedEof)?,
    ]);
    Ok(Query {
        header,
        question: Question {
            name,
            qtype,
            qclass,
        },
    })
}

/// Закодировать доменное имя в формат меток (label, label, 0).
fn encode_name(name: &str, out: &mut Vec<u8>) {
    for label in name.split('.') {
        if label.is_empty() {
            continue;
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
}

/// Бит AA (Authoritative Answer) в флагах DNS-заголовка.
const FLAG_AA: u16 = 0x0400;

/// Собрать ответ с одной A-записью на исходный question.
///
/// `ttl` — время жизни записи в секундах.
/// `authoritative` — выставить ли AA-флаг (для локальной зоны = true).
pub fn build_a_response(query: &Query, ip: Ipv4Addr, ttl: u32, authoritative: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);

    // Заголовок: тот же id, QR=1, RD копируем из запроса, RA=1, RCODE=0.
    out.extend_from_slice(&query.header.id.to_be_bytes());
    let rd = query.header.flags & 0x0100; // бит RD
    let aa = if authoritative { FLAG_AA } else { 0 };
    let flags: u16 = 0x8000 | aa | rd | 0x0080; // QR=1, [AA], RD, RA=1
    out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    out.extend_from_slice(&1u16.to_be_bytes()); // ANCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT

    // Question — повторяем как есть.
    encode_name(&query.question.name, &mut out);
    out.extend_from_slice(&query.question.qtype.to_be_bytes());
    out.extend_from_slice(&query.question.qclass.to_be_bytes());

    // Answer: имя через compression-указатель на question (offset 12 = 0xC00C).
    out.extend_from_slice(&[0xC0, 0x0C]);
    out.extend_from_slice(&TYPE_A.to_be_bytes());
    out.extend_from_slice(&CLASS_IN.to_be_bytes());
    out.extend_from_slice(&ttl.to_be_bytes());
    out.extend_from_slice(&4u16.to_be_bytes()); // RDLENGTH
    out.extend_from_slice(&ip.octets());

    out
}

/// Собрать ответ-ошибку (без answer-секции) с заданным RCODE.
///
/// `authoritative` — выставить ли AA-флаг (для NXDOMAIN локальной зоны = true).
pub fn build_error_response(query: &Query, rcode: u8, authoritative: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&query.header.id.to_be_bytes());
    let rd = query.header.flags & 0x0100;
    let aa = if authoritative { FLAG_AA } else { 0 };
    let flags: u16 = 0x8000 | aa | rd | 0x0080 | (rcode as u16 & 0x0F);
    out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    encode_name(&query.question.name, &mut out);
    out.extend_from_slice(&query.question.qtype.to_be_bytes());
    out.extend_from_slice(&query.question.qclass.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Собрать сырой запрос для теста.
    fn make_query(name: &str, qtype: u16) -> Vec<u8> {
        let mut q = Vec::new();
        q.extend_from_slice(&0x1234u16.to_be_bytes()); // id
        q.extend_from_slice(&0x0100u16.to_be_bytes()); // flags: RD=1
        q.extend_from_slice(&1u16.to_be_bytes()); // qdcount
        q.extend_from_slice(&0u16.to_be_bytes()); // ancount
        q.extend_from_slice(&0u16.to_be_bytes()); // nscount
        q.extend_from_slice(&0u16.to_be_bytes()); // arcount
        encode_name(name, &mut q);
        q.extend_from_slice(&qtype.to_be_bytes());
        q.extend_from_slice(&CLASS_IN.to_be_bytes());
        q
    }

    #[test]
    fn parse_domain_name() {
        let mut buf = Vec::new();
        encode_name("Router.LAN", &mut buf);
        let (name, pos) = parse_name(&buf, 0).unwrap();
        assert_eq!(name, "router.lan"); // приводится к нижнему регистру
        assert_eq!(pos, buf.len());
    }

    #[test]
    fn parse_full_query() {
        let raw = make_query("admin.lan", TYPE_A);
        let q = parse_query(&raw).unwrap();
        assert_eq!(q.header.id, 0x1234);
        assert!(q.header.is_query());
        assert_eq!(q.question.name, "admin.lan");
        assert_eq!(q.question.qtype, TYPE_A);
        assert_eq!(q.question.qclass, CLASS_IN);
    }

    #[test]
    fn reject_short_packet() {
        assert!(matches!(parse_header(&[0, 1, 2]), Err(DnsError::TooShort)));
    }

    #[test]
    fn reject_compression_in_question() {
        // Имя, начинающееся с compression-указателя 0xC0.
        let buf = [0xC0u8, 0x0C];
        assert!(matches!(parse_name(&buf, 0), Err(DnsError::BadName)));
    }

    #[test]
    fn reject_too_long_label() {
        // Метка длиной 64 (> 63) недопустима.
        let mut buf = vec![64u8];
        buf.extend(std::iter::repeat(b'a').take(64));
        buf.push(0);
        assert!(matches!(parse_name(&buf, 0), Err(DnsError::BadName)));
    }

    #[test]
    fn reject_invalid_label_chars() {
        let mut buf = Vec::new();
        // метка "ab cd" с пробелом — недопустимый символ
        buf.push(5u8);
        buf.extend_from_slice(b"ab cd");
        buf.push(0);
        assert!(matches!(parse_name(&buf, 0), Err(DnsError::BadName)));
    }

    #[test]
    fn build_a_response_layout_and_aa() {
        let raw = make_query("router.lan", TYPE_A);
        let q = parse_query(&raw).unwrap();
        let resp = build_a_response(&q, Ipv4Addr::new(192, 168, 4, 1), 60, true);

        // id сохранён
        assert_eq!(u16::from_be_bytes([resp[0], resp[1]]), 0x1234);
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        assert_eq!(flags & 0x8000, 0x8000); // QR=1
        assert_eq!(flags & 0x0400, 0x0400); // AA=1 (authoritative)
        // ANCOUNT = 1
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 1);
        // Последние 4 байта — IP
        let n = resp.len();
        assert_eq!(&resp[n - 4..], &[192, 168, 4, 1]);
    }

    #[test]
    fn non_authoritative_has_no_aa() {
        let raw = make_query("google.com", TYPE_A);
        let q = parse_query(&raw).unwrap();
        let resp = build_a_response(&q, Ipv4Addr::new(1, 2, 3, 4), 60, false);
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        assert_eq!(flags & 0x0400, 0); // AA не выставлен
    }

    #[test]
    fn build_error_has_no_answers() {
        let raw = make_query("nope.lan", TYPE_A);
        let q = parse_query(&raw).unwrap();
        let resp = build_error_response(&q, RCODE_NXDOMAIN, true);
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 0); // ANCOUNT=0
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        assert_eq!((flags & 0x000F) as u8, RCODE_NXDOMAIN);
        assert_eq!(flags & 0x0400, 0x0400); // AA=1
    }
}

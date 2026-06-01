# nanoDNS

`nanodns` — минимальный DNS-сервер для локальной Wi-Fi сети **TinyWifi** (проект 4STM4 TinyWifi).

Один бинарник, только Rust std, без внешних зависимостей.

## Что это

`nanodns` слушает UDP DNS-запросы и:

- отдаёт A-записи для локальной зоны `.lan`;
- резолвит имя роутера (`router.lan`);
- отдаёт статические записи из конфига;
- читает DHCP leases от `nanodhcp` и резолвит `hostname.lan` → IP;
- форвардит неизвестные домены на внешний (upstream) DNS;
- умеет простой captive-режим и опциональный cache.

## Зачем он TinyWifi

TinyWifi — это маленький Wi-Fi роутер (например, на Raspberry Pi Zero 2 W).
Ему нужен лёгкий DNS, который:

- знает локальные имена устройств из DHCP (`phone.lan`, `laptop.lan`);
- даёт удобные имена для админки (`router.lan`, `admin.lan`);
- форвардит всё остальное в интернет;
- помещается в один бинарник и почти не ест память/CPU.

## Почему без зависимостей

- предсказуемая и быстрая сборка под слабое железо;
- маленький бинарник, нет дерева крейтов и транзитивных уязвимостей;
- проще аудит и поддержка;
- хватает `std`: UDP-сокеты, парсинг байтов, `HashMap`.

Не используются `tokio`, `axum`, `serde`, `toml`, `hickory`, `anyhow`, `log` и т.п.

## Сборка

```sh
cargo build --release
```

> Требуется toolchain Rust (edition 2024) и системный линкер.
> На macOS нужны Command Line Tools (`xcode-select --install`).

## Запуск

```sh
# с конфигом
cargo run -- --config ./config.example

# или собранный бинарник
./target/release/nanodns --config ./config.example
```

Если `--config` не передан, используется `/etc/nanodns/config`.
Если файла нет — применяются встроенные дефолты (listen `0.0.0.0:5353`, зона `lan`,
router `router`/`192.168.4.1`, upstream `1.1.1.1:53`).

> Для разработки используется порт **5353**, потому что порт 53 требует root.

## Проверка

```sh
cargo build
cargo test
cargo run -- --config ./config.example
```

В другом терминале:

```sh
dig @127.0.0.1 -p 5353 router.lan A
dig @127.0.0.1 -p 5353 admin.lan A
dig @127.0.0.1 -p 5353 google.com A
```

`router.lan` и `admin.lan` вернутся из локальных данных, `google.com` будет
форварднут на upstream.

## Формат конфига

Простой `key=value`, без TOML/JSON. Строки с `#` — комментарии, пустые игнорируются.
См. [`config.example`](config.example).

| Ключ          | Назначение                                                        |
|---------------|-------------------------------------------------------------------|
| `listen`      | адрес и порт UDP (`0.0.0.0:5353`)                                  |
| `domain`      | локальная зона (`lan`)                                            |
| `router_name` | имя роутера; `router_name.domain` резолвится в `router_ip`         |
| `router_ip`   | IPv4 роутера                                                       |
| `upstream`    | внешний DNS (можно несколько строк, опрашиваются по очереди)       |
| `lease_file`  | путь к leases от `nanodhcp`                                        |
| `cache`       | `true`/`false` — включить кэш                                      |
| `captive`     | `true`/`false` — captive-режим                                    |
| `captive_ip`  | IPv4, который отдаётся в captive-режиме                            |
| `record`      | статическая запись: `record=имя,A,ip,ttl`                         |

## Формат leases

`nanodhcp` отдаёт текстовый файл, одна аренда на строку:

```text
aa:bb:cc:dd:ee:ff 192.168.4.23 phone 1780310000
11:22:33:44:55:66 192.168.4.42 laptop 1780310000
```

Поля: `MAC IP hostname expiry`. `nanodns` резолвит:

```text
phone.lan  -> 192.168.4.23
laptop.lan -> 192.168.4.42
```

Если lease-файл отсутствует или строка битая — сервер не падает, просто пропускает.

## Порядок резолва

1. captive mode (если включён);
2. `router_name.domain`;
3. статические записи из конфига;
4. leases из `lease_file`;
5. cache (если включён);
6. форвард на upstream;
7. иначе SERVFAIL (upstream недоступен) / NXDOMAIN (локальная зона без записи).

## Captive mode

Если `captive=true`, **любой** A/IN-запрос получает `captive_ip` (удобно для
страницы-перехвата). Запросы **не A** или **не IN** в captive-режиме
**форвардятся** на upstream (выбран простой вариант вместо NOTIMP).

## Текущие ограничения (v0.1)

- только один question на пакет;
- локально поддерживаются только A-записи;
- name compression в question **не** поддерживается; в ответах upstream
  compression приходит как есть (мы их не разбираем, а проксируем);
- TCP DNS не поддерживается;
- DNSSEC не поддерживается;
- DoH/DoT не поддерживается;
- IPv6/AAAA локально не поддерживается (AAAA форвардится наружу);
- cache простой, без LRU, TTL упрощён до 60 секунд;
- конфиг не TOML/JSON, а `key=value`.

## Структура

```
nanodns/
├─ Cargo.toml
├─ README.md
├─ config.example
└─ src/
   ├─ main.rs      # CLI, загрузка конфига, старт
   ├─ config.rs    # парсинг key=value
   ├─ dns.rs       # парсинг/сборка DNS-пакетов
   ├─ server.rs    # UDP-цикл
   ├─ resolver.rs  # логика выбора ответа
   ├─ leases.rs    # чтение leases от nanodhcp
   ├─ forward.rs   # форвардинг на upstream
   └─ cache.rs     # простой кэш
```

Дальше планируется интеграция с `nanodhcp` и web UI.

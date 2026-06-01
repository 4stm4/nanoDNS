# nanoDNS

[![CI](https://github.com/4stm4/nanoDNS/actions/workflows/ci.yml/badge.svg)](https://github.com/4stm4/nanoDNS/actions/workflows/ci.yml)
[![coverage](https://img.shields.io/badge/coverage-78.21%25-yellowgreen)](https://github.com/4stm4/nanoDNS)
[![version](https://img.shields.io/badge/version-0.2.0-blue)](https://github.com/4stm4/nanoDNS/releases)
[![license](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)
[![dependencies](https://img.shields.io/badge/dependencies-std%20only-success)](Cargo.toml)

`nanodns` is a minimal DNS server for small local networks.

A single binary, written in pure Rust `std`, with **zero external dependencies**.

## What it does

`nanodns` listens for UDP DNS queries and:

- serves A records for a configurable local zone (e.g. `.lan`);
- resolves the router/gateway name (e.g. `router.lan`);
- serves static records defined in the config;
- reads DHCP leases from a plain text file and resolves `hostname.lan` → IP;
- forwards unknown domains to upstream DNS servers;
- supports a simple captive mode and an optional response cache.

## Why

Small routers, home labs, and single-board computers (down to a Raspberry Pi
Zero 2 W) need a tiny DNS that:

- knows local device names coming from DHCP (`phone.lan`, `laptop.lan`);
- provides friendly names for local services (`router.lan`, `admin.lan`);
- forwards everything else to the internet;
- fits into one binary and uses almost no memory or CPU.

## Why zero dependencies

- predictable, fast builds on weak hardware;
- a tiny binary with no crate tree and no transitive vulnerabilities;
- easy to audit and maintain;
- `std` is enough: UDP sockets, byte parsing, `HashMap`.

No `tokio`, `axum`, `serde`, `toml`, `hickory`, `anyhow`, `log`, etc.

## Build

```sh
cargo build --release
```

> Requires a Rust toolchain (edition 2024) and a system linker.

## Run

```sh
# with a config file
cargo run -- --config ./config.example

# or the built binary
./target/release/nanodns --config ./config.example
```

If `--config` is not passed, `/etc/nanodns/config` is used.
If that file is missing, built-in defaults apply (listen `0.0.0.0:5353`,
zone `lan`, router `router`/`192.168.4.1`, upstream `1.1.1.1:53`).

> Development uses port **5353** because port 53 requires root.
> Note that on systems running mDNS (`avahi-daemon`), port 5353 may already be
> taken — pick another port via `listen=` in that case.

## Verify

```sh
cargo build
cargo test
cargo run -- --config ./config.example
```

In another terminal:

```sh
dig @127.0.0.1 -p 5353 router.lan A
dig @127.0.0.1 -p 5353 admin.lan A
dig @127.0.0.1 -p 5353 google.com A
```

`router.lan` and `admin.lan` are answered from local data; `google.com` is
forwarded to an upstream server.

## Config format

Simple `key=value`, no TOML/JSON. Lines starting with `#` are comments, blank
lines are ignored. See [`config.example`](config.example).

| Key           | Purpose                                                           |
|---------------|-------------------------------------------------------------------|
| `listen`      | UDP address and port (`0.0.0.0:5353`)                             |
| `domain`      | local zone (`lan`)                                                |
| `router_name` | router name; `router_name.domain` resolves to `router_ip`         |
| `router_ip`   | router IPv4                                                        |
| `upstream`    | upstream DNS (multiple lines allowed, tried in order)             |
| `lease_file`  | path to the DHCP lease file                                       |
| `cache`       | `true`/`false` — enable the response cache                        |
| `captive`     | `true`/`false` — captive mode                                     |
| `captive_ip`  | IPv4 returned in captive mode                                     |
| `record`      | static record: `record=name,A,ip,ttl`                            |

## Lease file format

A plain text file, one lease per line:

```text
aa:bb:cc:dd:ee:ff 192.168.4.23 phone 1780310000
11:22:33:44:55:66 192.168.4.42 laptop 1780310000
```

Fields: `MAC IP hostname expiry`. `nanodns` resolves:

```text
phone.lan  -> 192.168.4.23
laptop.lan -> 192.168.4.42
```

If the lease file is missing or a line is malformed, the server does not crash —
it simply skips it.

## Resolution order

1. captive mode (if enabled);
2. `router_name.domain`;
3. static records from the config;
4. leases from the lease file;
5. cache (if enabled);
6. forward to upstream;
7. otherwise SERVFAIL (upstream unreachable) / NXDOMAIN (local zone, no record).

## Captive mode

When `captive=true`, **every** A/IN query returns `captive_ip` (handy for a
captive portal page). Queries that are **not A** or **not IN** are **forwarded**
to upstream (a simple choice instead of returning NOTIMP).

## Current limitations (v0.2)

- one question per packet only;
- only A records are served locally;
- name compression is **not** supported in the question; upstream responses are
  proxied as-is (we do not parse their compression);
- no TCP DNS;
- no DNSSEC;
- no DoH/DoT;
- no local IPv6/AAAA (AAAA is forwarded upstream);
- the cache is simple, no LRU, TTL simplified to 60 seconds;
- the config is `key=value`, not TOML/JSON.

## Layout

```
nanodns/
├─ Cargo.toml
├─ README.md
├─ config.example
└─ src/
   ├─ main.rs      # CLI, config loading, startup
   ├─ config.rs    # key=value parser
   ├─ dns.rs       # DNS packet parsing/building
   ├─ server.rs    # UDP loop
   ├─ resolver.rs  # answer-selection logic
   ├─ leases.rs    # DHCP lease reading
   ├─ forward.rs   # upstream forwarding
   └─ cache.rs     # simple cache
```

## License

[AGPL-3.0](LICENSE).

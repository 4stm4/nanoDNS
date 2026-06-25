# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0]

### Added
- Domain blocking (ad/tracking sinkhole): new `block_file` option points to a
  list of domains (one per line, `#` comments, `*.domain` wildcards). Blocked
  names are answered with `block_response` — either a sinkhole IP (default
  `0.0.0.0`) or `NXDOMAIN` — and are never forwarded or cached.
- The block file is hot-reloaded by mtime (same mechanism as leases), so lists
  updated by an external tool are picked up without a restart. Large lists
  (StevenBlack/AdGuard, ~150k domains) are stored in a `HashSet` for O(1) lookup.

### Changed
- Resolution order: blocklist is checked after local records (so your own
  services can never be blocked) and before cache/forwarding.

## [0.3.0]

### Added
- Hot-reload of the lease file: leases are re-read when the file's mtime changes
  (checked at most once every 2 seconds), so clients that join after startup are
  resolved without restarting the service.
- Lease expiry handling: a `Lease` now carries `expires_at`; expired leases are
  skipped both on load and on lookup (`expires_at == 0` means never expires).
- Bounded response cache: new `cache_max_entries` and `cache_ttl` options. On
  insertion the cache drops expired entries and evicts entries once the limit is
  reached.
- Concurrent request handling: each request is served on its own thread so a slow
  upstream no longer blocks the whole server. New `max_inflight` option caps the
  number of concurrent worker threads (requests over the cap run inline).
- Authoritative answer flag: local A answers and local NXDOMAIN responses now set
  the `AA` bit; forwarded responses are passed through unchanged.

### Changed
- Cache key now includes the query class (`name:qtype:qclass`).
- Configured `domain`, `router_name` and static record names are normalized
  (lower-cased, trailing dot stripped).

### Fixed
- Stricter DNS name parsing: per-label length is limited to 63 octets, labels are
  restricted to `a-z 0-9 - _`, and a leading/trailing hyphen is rejected.

## [0.2.0]

### Added
- README rewritten in English; project documented as a standalone product.
- Status badges (CI, coverage, version, license, dependencies).
- GitHub Actions CI workflow (fmt + build + test).
- `license` field (AGPL-3.0) in `Cargo.toml`.

## [0.1.0]

### Added
- Initial minimal UDP DNS server: header/question parsing, A record responses,
  local `.lan` zone, static records, DHCP lease resolution, upstream forwarding,
  a simple cache and a captive mode.
- Zero external dependencies (Rust `std` only).

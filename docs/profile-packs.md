# Versioned ECU profile packs

Wideband loads vehicle **profile packs** from JSON files under `profiles/` (next to `wideband.exe`, `./profiles`, `~/.wideband/profiles`, or `WIDEBAND_PROFILE_DIRS`). Each pack describes bus wiring, diagnostic IDs, session types, and **references** into other config files — it never embeds seed-key masks or checksum secrets.

Community **calibration definition packs** (XDF/A2L/INI) and native **plugins** are documented in [`community-packs-and-plugins.md`](community-packs-and-plugins.md).

## Layout

| Path | Role |
| --- | --- |
| `profiles/*.json` | One pack per file (`pack_version`: **1**) |
| `security-config.json` | Seed-key algorithms and checksum ranges (`refs.security` / `refs.checksum` point here) |
| `flash-layout.json` | ROM origin, stock read, cal segments (`refs.flash_layout` points here) |

## Pack schema (v1)

Required fields:

- `id`, `name`, `platform`
- `bus`: `can_isotp` \| `kline` \| `vpw`
- `diagnostic`: `uds` \| `kwp`
- `bitrate`, `j2534` (`protocol`, `connect_flags`, `extended_id`)
- `can` (`request_id`, `response_id`, `filter_mask` as hex strings)
- `kline` (`tester`, `ecu`)
- `memory` (`max_chunk`, `address_bytes`, `size_bytes`)
- `sessions` (`programming`, `read`)
- `refs`: `security`, `flash_layout`, `checksum` — string ids only

Forbidden in packs (loader rejects): `mask`, `key`, `keyId`, `algorithm`, `checksums`, `configured`, and similar security material. Keys stay in `security-config.json`; unconfigured rows **fail closed** at runtime.

## Shipped examples

- `profiles/tacoma-can.json` — 2017 Tacoma / Denso CAN (UDS on ISO-TP, 500 kbit/s)
- `profiles/tacoma-kline.json`
- `profiles/silverado-can.json`
- `profiles/silverado-vpw.json`

Add a new family by copying a pack, adjusting transport IDs, and adding matching rows in `flash-layout.json` and `security-config.json` (still with `configured: false` until you deliberately configure bench keys).

## UI

The profile dropdown is populated from discovered packs. If `profiles/` is missing or empty, the app falls back to built-in defaults (same four profiles).

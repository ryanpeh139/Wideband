# TCM profile packs (Transmission)

Wideband Motorsport Studio loads **TCM** packs from `profiles/tcm/*.json` (beside the binary or `./profiles/tcm` in development). Each pack pairs with an ECM profile via `paired_ecm` and references `security-config.json` and `flash-layout.json` by id — never embed keys or checksum secrets.

## Shipped stubs

| Pack id | Paired ECM | Bus | Notes |
| --- | --- | --- | --- |
| `tacoma-can-tcm` | `tacoma-can` | CAN 0x7E1 / 0x7E9 | A750E stub |
| `tacoma-kline-tcm` | `tacoma-kline` | K-Line | Identify not implemented yet |
| `silverado-can-tcm` | `silverado-can` | Extended CAN 0x18DA28F1 / 0x18DAF128 | 6L80 stub |
| `silverado-vpw-tcm` | `silverado-vpw` | VPW | Transport stub |

## UI

- **Transmission** tab: TCM identify (read-only), definition-tagged shift / pressure / TCC tables from the loaded XDF, and a **fail-closed** TCM flash button (blocked until `security-config.json`, checksum rows, and non-empty `cal_flash_segments` are configured; execution wiring is a later milestone).

## Extension

Copy a pack, set `paired_ecm`, CAN IDs, and `transmission_family`, then add matching `refs` rows in `security-config.json` and `flash-layout.json`.

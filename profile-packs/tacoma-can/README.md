# Tacoma (Denso) profile pack — checksum routines

Bench tests use `tests/security-config-test.json` (`sum32` over `0x0..0x3`, store at `0x4`).

Production Tacoma CAN checksum windows belong in repo-root `security-config.json` with `configured: true` once validated against a stock `.bin` / `.cuw` image.

**Extension point:** algorithms `denso_block_sum` and `delco_crc32` are registered in `src/checksum_routine.rs` and return explicit errors until a pack entry maps them to real ranges.

# Silverado (Delco) profile pack — stub

GM seed-key rows live in `security-config.json` (`gmKeys` + profile `keyId`).

Checksum correction for Delco ECMs will use the `delco_crc32` routine id once flash-layout and stock ROM ranges are validated. Until then, cal flash remains fail-closed when no configured checksum block applies to the active image.

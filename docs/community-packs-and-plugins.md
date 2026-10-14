# Community definition packs and `cdylib` plugins

Wideband discovers **ECU profile packs**, **calibration definition packs**, and native **plugins** from shipped directories and optional user paths. Packs only carry **references** (profile ids, file paths, plugin ids). Seed-key masks, checksum ranges, and algorithms stay in `security-config.json` — the same fail-closed rules as [`profile-packs.md`](profile-packs.md).

## Directory layout

| Location | Contents |
| --- | --- |
| `profiles/*.json` | Shipped ECU profile packs (v1) |
| `definition-packs/<id>/pack.json` | Community calibration/logger packs |
| `definition-packs/<id>/` | XDF, A2L/DAMOS, INI, or custom sidecar files |
| `plugins/<id>/plugin.json` | Plugin manifest |
| `plugins/<id>/bin/` | Platform library (`libwideband_noop.so`, `.dll`, …) |
| `profile-packs/` | Checksum routine notes (not loaded at runtime) |

### User / extra paths

| Variable | Purpose |
| --- | --- |
| `WIDEBAND_PROFILE_DIRS` | Extra ECU profile roots (`;` or `,` separated) |
| `WIDEBAND_DEFINITION_PACK_DIRS` | Extra definition-pack roots |
| `WIDEBAND_PLUGIN_DIRS` | Extra plugin roots |
| `~/.wideband/` (or `%APPDATA%\Wideband\`) | Default user `profiles/`, `definition-packs/`, `plugins/` |

Discovery order: executable directory → repo `./` paths → env overrides → user data. **First profile id wins** across roots; definition packs with duplicate ids are skipped with a warning.

## Definition pack schema (v1)

`pack.json` fields:

- `pack_version`: **1**
- `id`, `title`
- `profile_ids`: ECU profiles this pack applies to (e.g. `tacoma-can`)
- `primary`: id of the primary calibration file in `files`
- `files[]`: `{ "id", "role", "path", "optional"? }` where `role` is `xdf` \| `a2l` \| `ini` \| `custom`

Optional `plugins` block (plugin ids only):

```json
"plugins": {
  "checksum": "wideband-noop",
  "protocol": "wideband-noop",
  "patch": "wideband-noop",
  "manager": "wideband-noop"
}
```

If a pack or profile references a plugin id, that plugin **must** load successfully or the pack/profile is rejected (fail-closed).

Forbidden in packs (same as ECU profile packs): `mask`, `key`, `checksums`, `configured`, `algorithm`, etc.

### Shipped sample

- [`definition-packs/tacoma-can-bench/`](../definition-packs/tacoma-can-bench/) — XDF for Tacoma CAN bench tables (`tests/fixtures/tacoma-bench-table.xdf`).

## Plugin manifest (`plugin.json`)

```json
{
  "plugin_version": 1,
  "id": "wideband-noop",
  "library": "wideband_noop"
}
```

The host loads `bin/libwideband_noop.so` (or `wideband_noop.dll` on Windows) and verifies:

- `wb_plugin_get_descriptor` → API version **1** and matching `id`
- Optional exports: `wb_checksum_hook`, `wb_protocol_hook`, `wb_patch_hook`, `wb_manager_hook`

### Sample crate

Build the no-op reference plugin:

```
cargo build -p wideband-noop
cp target/debug/libwideband_noop.so plugins/wideband-noop/bin/
```

Sources: [`plugins/wideband-plugin-api/`](../plugins/wideband-plugin-api/) (ABI), [`plugins/wideband-noop/`](../plugins/wideband-noop/).

Checksum hooks run **after** built-in `security-config.json` stamping when a profile or pack references `plugins.checksum`.

## ECU profile pack extensions

Optional `plugins` object on `profiles/*.json` uses the same ids. Profiles that reference missing plugins are omitted from the catalog with an error note (built-in defaults still available if no valid packs load).

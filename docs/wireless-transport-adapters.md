# Wireless and Ethernet transport adapters (Motorsport Studio)

Rust Motorsport Studio discovers **four transport kinds** alongside the existing USB J2534 path:

| Kind | Discovery | Connect path |
| --- | --- | --- |
| **Virtual bench** | Always listed first | In-process `VirtualEcuSimulator` (not a vehicle cable) |
| **USB J2534** | Windows registry + known DLL fallbacks | Unchanged: `J2534Host.exe` 32-bit IPC when present, else in-process `libloading` |
| **Bluetooth serial** | `WIDEBAND_BT_SERIAL` env (comma-separated ports) | `WirelessPassthruAdapter` stub until an RFCOMM/BLE-UART backend is linked |
| **Ethernet gateway** | `WIDEBAND_ETH_GATEWAYS=host:port,...` | TCP client using the same newline JSON protocol as `J2534Host.exe` |

Implementation: `src/transport.rs` (`TransportRegistry`, `TransportDevice`, `WirelessPassthruAdapter`).

## Environment variables

| Variable | Example | Purpose |
| --- | --- | --- |
| `WIDEBAND_BT_SERIAL` | `COM9` or `/dev/rfcomm0` | RFCOMM / BLE-UART COM port(s) for a serial OBD bridge |
| `WIDEBAND_ETH_GATEWAYS` | `192.168.4.1:35000,10.0.0.8:35000` | TCP endpoints for a JSON-line PassThru gateway |
| `WIDEBAND_GATEWAY_DLL` | `C:\path\to\esp32.dll` | DLL path sent in `OPEN` on Ethernet gateways (default: `gateway`) |

After changing env vars, use **Rescan** in the device combo (disconnect first if already connected).

## UI labels and error states

Device rows use prefixed labels:

- `[USB J2534]`, `[Bluetooth]`, `[Ethernet]`, `[Virtual]`

Suffixes:

- **— offline** — Ethernet probe failed (host:port not accepting TCP)
- **— DLL not found** — registry J2534 entry without `FunctionLibrary` on disk
- **— not configured** — placeholder row; set env vars above
- **— error** — last open attempt failed (header chip shows detail)

The header **OFFLINE** chip shows the last connection error when disconnected.

## Adapter contract (`WirelessPassthruAdapter`)

Vendor SDKs plug in by implementing:

```rust
pub trait WirelessPassthruAdapter: Send + Sync {
    fn open_bluetooth_serial(&self, port_name: &str) -> Result<Box<dyn PassThruDevice>, String>;
    fn open_ethernet_gateway(&self, host: &str, port: u16) -> Result<Box<dyn PassThruDevice>, String>;
}
```

The stock `StubWirelessAdapter`:

- **Bluetooth** — returns a clear error until a serial framing backend is wired.
- **Ethernet** — opens TCP and speaks `OPEN`, `CONNECT`, `FILTER`, `READ`, `WRITE`, `VERSION`, `CLOSE`, `QUIT` (same JSON shape as `J2534Host.exe` on stdout).

`PassThruDevice` from `j2534.rs` is unchanged; ISO-TP / UDS / flash code does not branch on transport.

## J2534Host IPC (unchanged)

64-bit `wideband` on Windows still spawns **32-bit `J2534Host.exe`** beside the executable for USB PassThru DLLs. Wireless transports do **not** replace or merge into that process.

## Bench-only virtual ECU

The virtual loopback device remains explicitly labeled and is not treated as a wireless or USB cable. Security keys for virtual bench use the existing zero-mask registration path only when that device is selected.

# CLAUDE.md — laundry-monitor

A bare-metal ESP32 Rust project that detects laundry cycle completion via an
SW-420 vibration sensor and sends an HTTP push notification to a local
[ntfy.sh](https://ntfy.sh) instance running in Docker.

---

## Hardware

| Component        | Detail                                             |
|------------------|----------------------------------------------------|
| MCU              | ESP32 (Xtensa LX6)                                 |
| Sensor           | SW-420 vibration sensor module (one per appliance  |
| Sensor pin       | GPIO25 (active-high, internal pull-down)           |
| Sensor pin       | GPIO26 (active-high, internal pull-down)           |
| Power            | USB or 3.3 V rail                                  |

**SW-420 wiring:**
- `VCC` → 3.3 V
- `GND` → GND
- `D0`  → GPIO25/GPIO26

The sensor output goes **HIGH** when vibrating and **LOW** when still.

---

## Project Layout

```
laundry-monitor/
├── build.rs                  # Env var baking + linker error helper
├── Cargo.toml
├── Cargo.lock
├── CLAUDE.md                 # This file
├── rust-toolchain.toml       # Pins the `esp` toolchain channel
├── setup_environment.sh      # One-shot dev environment bootstrap
├── .cargo/
│   └── config.toml           # Target, runner, rustflags, build-std
├── docker/
│   ├── Dockerfile
│   └── server.yml            # ntfy Docker Compose config
└── src/
    ├── lib.rs                # #![no_std] + mod declarations
    ├── bin/
    │   └── main.rs           # Entry point: init, WiFi, poll loop
    ├── wifi_adapter.rs       # WifiDevice → smoltcp::phy::Device bridge
    ├── notify.rs             # HTTP POST to ntfy, socket buffers
    └── utils.rs              # noop_waker, blocking_delay, const parsers
```

---

## Environment Setup

Run the bootstrap script once on a fresh Linux machine (tested on Ubuntu 24):

```bash
chmod +x setup_environment.sh
./setup_environment.sh
```

This installs:
- System dependencies (`gcc`, `build-essential`, `curl`, `pkg-config`, `git`)
- `rustup` (if absent)
- `espup` (built from source)
- The Xtensa toolchain via `espup install` → generates `~/export-esp.sh`
- `esp-generate` and `espflash` via `cargo install`

After the script completes, add the ESP environment to your shell permanently:

```bash
echo '. $HOME/export-esp.sh' >> ~/.bashrc   # or ~/.zshrc
source ~/.bashrc
```

For subsequent shells, source it manually if not added to your profile:

```bash
. ~/export-esp.sh
```

---

## Configuration

### `.cargo/config.toml`

```toml
[target.xtensa-esp32-none-elf]
runner = "espflash flash --monitor --chip esp32"

[build]
rustflags = ["-C", "link-arg=-nostartfiles"]
target    = "xtensa-esp32-none-elf"

[unstable]
build-std = ["core", "alloc"]
```

### `.env` (gitignored)

Create a `.env` file in the project root. `build.rs` reads this at compile
time and bakes the values into the binary:

```dotenv
WIFI_SSID=your-network-name
WIFI_PASSWORD=your-network-password
HOST_IP=192.168.1.100        # LAN IP of the machine running the ntfy container
NFTY_PORT=80                 # Port ntfy is listening on

WASHER_GPIO=25               # GPIO pin for the washer SW-420 sensor
WASHER_TOPIC=washer          # ntfy topic for washer notifications
WASHER_IDLE_TIMEOUT_SECS=30  # Seconds of stillness before alerting

DRYER_GPIO=26                # GPIO pin for the dryer SW-420 sensor
DRYER_TOPIC=dryer            # ntfy topic for dryer notifications
DRYER_IDLE_TIMEOUT_SECS=60   # Seconds of stillness before alerting
```

> **Note:** Any variable already exported in the shell takes precedence over
> `.env`. This lets CI override values without touching the file.

`HOST_IP` is split into four octets at compile time (`HOST_IP_0`..`HOST_IP_3`)
and reconstructed into an `Ipv4Address` constant — no runtime string parsing
occurs on the device.

---

## Build & Flash

```bash
# Build only
cargo build --release

# Flash and open serial monitor (uses espflash via the configured runner)
cargo run --release
```

The serial monitor will print structured log lines:

```
[INFO] Connecting to 'your-network'...
[INFO] WiFi connected.
[INFO] Waiting for DHCP...
[INFO] IP: ...
[INFO] Network ready.
[INFO] Laundry monitor started — watching Washer (GPIO25) and Dryer (GPIO26).
[DEBUG] Washer -> VIBRATING
[DEBUG] Washer -> STILL
[DEBUG] Washer IDLE (12/30 s without vibration)
[INFO] Washer cycle complete — sending ntfy notification...
[INFO] ntfy notification sent (topic: 'washer').
```

---

## ntfy Docker Setup

The `docker/` directory contains the Compose config for a local ntfy instance.

```bash
cd docker
docker compose -f server.yml up -d
```

The ESP32 POSTs to `http://<HOST_IP>:<NFTY_PORT>/<TOPIC>` (where `<TOPIC>` is
`WASHER_TOPIC` or `DRYER_TOPIC` depending on which appliance completed) using a
minimal HTTP/1.0 request. HTTP/1.0 is used deliberately — the server closes
the connection after responding, so no header parsing is needed to detect
end-of-response.

Subscribe to notifications via the ntfy mobile app or web UI at
`http://<HOST_IP>:<NFTY_PORT>`.

---

## Architecture

### Toolchain

The ESP32's Xtensa architecture is not supported by upstream Rust. The `esp`
toolchain channel (managed by `espup`) provides a patched compiler and
`xtensa-esp32-none-elf` target.

```toml
# rust-toolchain.toml
[toolchain]
channel = "esp"
```

### No OS / Bare Metal

The project runs bare-metal (`#![no_std]`, `#![no_main]`) with no async
executor. The main loop busy-polls the sensor every 5 ms and drives the
network stack via `stack.work()` on each iteration.

### Key Crates

| Crate                    | Role                                              |
|--------------------------|---------------------------------------------------|
| `esp-hal`                | Bare-metal HAL (GPIO, clocks, RNG, timers)        |
| `esp-radio`              | WiFi driver (STA mode)                            |
| `esp-rtos`               | Scheduler required by `esp-radio`                 |
| `esp-alloc`              | Heap allocator (`alloc` crate support)            |
| `esp-bootloader-esp-idf` | IDF-compatible bootloader descriptor              |
| `smoltcp`                | Embedded TCP/IP stack (IPv4, DHCP, TCP)           |
| `blocking-network-stack` | Blocking socket abstraction over `smoltcp`        |
| `heapless`               | Fixed-capacity `String` for HTTP request building |
| `esp-println`            | `println!` over UART for serial logging           |

### Module Responsibilities

**`wifi_adapter.rs`** — Bridges `esp_radio::wifi::WifiDevice` (which implements
`embassy_net_driver::Driver`) to `smoltcp::phy::Device`. A noop waker is used
because the network stack is driven by busy-wait polling, not an async executor.

**`notify.rs`** — Owns the static TCP socket buffers (`SOCK_RX`, `SOCK_TX`) and
the `send_ntfy_notification()` function. Takes `host_ip`, `port`, and `topic`
as parameters to remain self-contained. Reads the response to completion (5s
budget) before disconnecting, even though the response body is discarded.

**`utils.rs`** — Pure, dependency-free helpers:
- `noop_waker()` — used by `wifi_adapter` for polling without an executor
- `blocking_delay()` — busy-wait; appropriate for single-task no-OS design
- `octet()` / `parse_u16()` — `const fn` parsers for compile-time IP/port baking

**`build.rs`** — Does two things at build time:
1. Reads `.env` (or shell env) and emits `cargo:rustc-env` for each variable
2. Registers itself as the linker's `--error-handling-script` to emit
   human-friendly diagnostics for common undefined-symbol errors (missing
   `defmt`, `esp-alloc`, `linkall.x`, etc.)

### State Machine (main loop)

```
         ┌─────────────────────────────────────────────────┐
         │                  IDLE / WAITING                  │
         │        last_vibration = None, alert_sent = false │
         └────────────────────┬────────────────────────────┘
                              │ sensor goes HIGH
                              ▼
         ┌─────────────────────────────────────────────────┐
         │                   VIBRATING                      │
         │  last_vibration = Some(now), alert_sent = false  │
         └────────────────────┬────────────────────────────┘
                              │ sensor stays LOW for IDLE_TIMEOUT (30s)
                              ▼
         ┌─────────────────────────────────────────────────┐
         │               CYCLE COMPLETE                     │
         │         send_ntfy_notification()                 │
         │              alert_sent = true                   │
         └────────────────────┬────────────────────────────┘
                              │ sensor goes HIGH again
                              ▼
                         (back to VIBRATING, alert_sent = false)
```

---

## Known Limitations & Future Work

- **No vibration debounce** — a single `is_high` reading resets the 30s idle
  timer. Brief spurious pulses (footsteps, HVAC) can delay the alert. A
  minimum sustained vibration window (e.g. N consecutive HIGH polls) should be
  added.
- **Duplicate alert risk** — if a spurious vibration arrives while the ntfy
  POST is in-flight (~5s window), `alert_sent` is cleared and a second
  notification will fire. Guard by only clearing `alert_sent` after a
  confirmed new cycle (sustained vibration after the idle timeout fired).
- **Busy-wait power consumption** — `blocking_delay` burns CPU continuously.
  For battery-powered deployments, replace with a low-power delay from
  `esp-hal`.
- **Single notification type** — the message body is hardcoded. Future work
  could include cycle duration, vibration intensity bucketing, or configurable
  messages.
- **No TLS** — communication to ntfy is plaintext HTTP. Acceptable for a local
  LAN deployment; would need TLS for internet-facing ntfy instances.

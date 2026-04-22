# laundry-monitor

A small embedded Rust project for the ESP32 that tells you when your laundry
is done. It watches an SW-420 vibration sensor attached to your washing machine
or dryer, and sends a push notification to your phone when the machine goes
quiet.

Notifications are delivered through [ntfy.sh](https://ntfy.sh), a simple
open-source notification service you run locally in Docker — no cloud accounts
or API keys required.

---

## How It Works

The ESP32 sits on top of (or next to) your washing machine with the vibration
sensor touching the surface. While the machine is running, the sensor detects
movement. Once the machine has been still for 30 seconds(configurable --passed via .env currently), the ESP32 sends an
HTTP notification to your local ntfy server, which forwards it to the ntfy app
on your phone.

---

## What You Need

**Hardware**

- ESP32 development board
- SW-420 vibration sensor module
- Three jumper wires
- USB cable for flashing

**Software**

- Tested on Ubuntu 22.04.5 LTS
- Docker, for running the ntfy notification server ([nfty docker image](https://hub.docker.com/r/binwiederhier/ntfy))
- The ntfy app on your phone ([iOS](https://apps.apple.com/us/app/ntfy/id1625396347) / [Android](https://play.google.com/store/apps/details?id=io.heckel.ntfy))

---

## Wiring

Connect the SW-420 sensor to the ESP32 as follows:

| Sensor Pin | ESP32 Pin |
|------------|-----------|
| VCC        | 3.3 V     |
| GND        | GND       |
| D0         | GPIO25    |

Attach the sensor to the top or side of your machine so it makes firm contact
with the surface.

---

## Setup

### Before you begin: grant serial port access

When you plug an ESP32 into your computer over USB, Linux exposes it as a
serial device — typically `/dev/ttyUSB0` or `/dev/ttyACM0`. By default, only
the `root` user and members of the `dialout` group can read from and write to
these devices. If your user is not in that group, `espflash` will fail silently
or report that it cannot find the device, even when the board is clearly
connected.

Add your user to the `dialout` group now, before running anything else:

```bash
sudo usermod -aG dialout $USER
```

Then log out and log back in. Group membership changes do not take effect in
your current session — you must start a new one. You can confirm it worked by
running `groups` and checking that `dialout` appears in the output.

This only needs to be done once per user account on a given machine.

### 1. Prepare your development environment

Run the setup script once to install everything the project needs:

```bash
chmod +x setup_environment.sh
./setup_environment.sh
```

What the script does:

**System packages (via `apt-get`)**

Installs `gcc`, `build-essential`, `curl`, `pkg-config`, and `git`. These are
standard C build tools that the Rust ESP32 toolchain depends on internally,
even though the project itself is written in Rust.

**Rust (via `rustup`)**

Installs the Rust toolchain manager if it is not already present on your
machine. If you already have Rust installed, this step is skipped.

**`espup` (built from source via `cargo install`)**

`espup` is Espressif's official tool for managing the ESP32 Rust toolchain.
The ESP32 uses a processor architecture (Xtensa) that the standard Rust
compiler does not support, so Espressif maintains their own patched compiler.
`espup` downloads and manages that compiler for you.

**The Xtensa toolchain (via `espup install`)**

Runs `espup install`, which downloads the patched Rust compiler and supporting
tools needed to compile code for the ESP32. This step can take several minutes
on a slow connection. It also generates a file at `~/export-esp.sh` that sets
the required environment variables.

**`esp-generate` and `espflash` (via `cargo install`)**

- `esp-generate` — a project scaffolding tool for starting new ESP32 Rust projects.
- `espflash` — the tool used to flash compiled firmware onto the ESP32 over USB and open the serial monitor.

**Host IP detection**

The script attempts to detect your machine's local network IP address and
write it to a `.env` file as `HOST_IP`. This saves you from having to look it
up manually. You should verify it is correct before flashing, as it is the
address the ESP32 will use to reach the ntfy notification server.

---

When the script finishes, add the ESP environment to your shell so it persists
across sessions:

```bash
echo '. $HOME/export-esp.sh' >> ~/.bashrc
source ~/.bashrc
```

### 2. Start the ntfy notification server

```bash
cd docker
docker compose -f server.yml up -d
```

The ntfy web interface will be available at `http://<your-machine-ip>:<port>`.
Open the ntfy app on your phone, add a server pointing to that address, and
subscribe to both your washer and dryer topic names.

### 3. Create a `.env` file

In the project root, create a file named `.env` with your network details:

```
WIFI_SSID=your-network-name
WIFI_PASSWORD=your-network-password
HOST_IP=192.168.1.100
PORT=80

WASHER_GPIO=25
WASHER_TOPIC=washer
WASHER_IDLE_TIMEOUT_SECS=30

DRYER_GPIO=26
DRYER_TOPIC=dryer
DRYER_IDLE_TIMEOUT_SECS=60
```

- `HOST_IP` is the local IP address of the machine running the ntfy Docker container.
- `WASHER_TOPIC` and `DRYER_TOPIC` can be any word — they just need to match the topics you subscribed to in the ntfy app.
- `WASHER_IDLE_TIMEOUT_SECS` and `DRYER_IDLE_TIMEOUT_SECS` control how long each appliance must be still before a notification is sent. Dryers typically run longer cycles so a higher value reduces false alerts.

These values are compiled directly into the firmware, so no credentials are
stored on the device at runtime.

### 4. Flash the ESP32

Plug in your ESP32 via USB, then run:

```bash
cargo run --release
```

This compiles the firmware, flashes it to the board, and opens a serial
monitor so you can watch what the device is doing.

---

## Using It

Once flashed, the ESP32 runs on its own — no computer needed. Power it from
any USB charger.

On startup it connects to your WiFi network and waits for vibration. You will
see output like this in the serial monitor:

```
[INFO] Connecting to 'your-network'...
[INFO] WiFi connected.
[INFO] Waiting for DHCP...
[INFO] Network ready.
[INFO] Laundry monitor started — watching Washer (GPIO25) and Dryer (GPIO26).
```

When a machine starts:

```
[DEBUG] Washer -> VIBRATING
```

When a machine finishes and goes quiet:

```
[DEBUG] Washer -> STILL
[DEBUG] Washer IDLE (10/30 s without vibration)
[DEBUG] Washer IDLE (20/30 s without vibration)
[INFO] Washer cycle complete — sending ntfy notification...
[INFO] ntfy notification sent (topic: 'washer').
```

A notification will appear on your phone shortly after.

The monitor resets automatically and is ready to detect the next cycle
immediately after sending the alert.

---

## Project Structure

```
laundry-monitor/
├── build.rs                  # Reads .env and bakes values into the firmware
├── Cargo.toml                # Dependencies
├── rust-toolchain.toml       # Pins the ESP32 Rust toolchain
├── setup_environment.sh      # Development environment setup
├── .cargo/
│   └── config.toml           # Build target and flash configuration
├── docker/
│   ├── Dockerfile
│   └── server.yml            # ntfy Docker Compose file
└── src/
    ├── lib.rs                # Module declarations
    ├── bin/
    │   └── main.rs           # Startup, WiFi, and main sensor loop
    ├── appliance.rs          # Per-appliance state machine
    ├── wifi_adapter.rs       # WiFi driver glue code
    ├── notify.rs             # Sends the HTTP notification
    └── utils.rs              # Shared helper functions
```

---

## Troubleshooting

**The device connects to WiFi but never sends a notification.**
Check that `HOST_IP` in your `.env` points to the machine running Docker, and
that the ntfy container is running. Confirm the port matches what Docker is
exposing.

**A notification fires too early or too late.**
Adjust `WASHER_IDLE_TIMEOUT_SECS` or `DRYER_IDLE_TIMEOUT_SECS` in your `.env`
and reflash. No source code changes are needed.

**Spurious notifications when nobody is doing laundry.**
The sensor may be picking up footsteps or other household vibration. Try
repositioning it, or increase the relevant idle timeout to require a longer
quiet period before alerting.

**`cargo run` cannot find the device.**
Make sure the ESP32 is connected via USB and that your user has serial port
access. See the note in the Setup section above.

## TODO
**Rename NFTY references to NTFY(lol)**
**Implement debouncing to eliminate false positives.**
**Add logic for magnetic read sensors (door state: OPEN/CLOSED).**
**If door is OPEN, we shouldn't be sending alerts. System should only be active when the door is CLOSED and vibrations are detected.**
**Logic for handling interuptions such as: DOOR is opened while cycle is active, pauses mid-cycle, with the ability to continue when CLOSED.**

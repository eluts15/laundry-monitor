Create a new bare-metal Rust ESP32 sensor monitoring project based on the laundry-monitor pattern.

Ask the user for:
1. Project name (used for the directory and binary name)
2. Target chip (default: esp32)
3. Sensor GPIO pin number
4. Whether the sensor is active-high or active-low (or unknown — offer to add diagnostic logging)
5. Idle timeout in minutes (how long with no signal before alerting)
6. A short description of what the sensor detects

Then:
1. Run `esp-generate --chip <chip> <project-name> --headless` to scaffold the project
2. Update `Cargo.toml` to add `esp-println = { version = "0.13", features = ["<chip>"] }`
3. Replace `src/bin/main.rs` with an implementation that follows this pattern:
   - `#![no_std]` / `#![no_main]`
   - GPIO configured as `Input` with appropriate `Pull::Down` (active-high) or `Pull::Up` (active-low)
   - Poll loop at 5 ms intervals using `blocking_delay` + `Instant`/`Duration` from `esp_hal::time`
   - Track `last_signal: Option<Instant>` — updated on every detected pulse
   - Log state transitions (signal detected, signal lost, periodic idle countdown, alert)
   - Alert via `println!` when `last_signal.elapsed() >= IDLE_TIMEOUT`
   - `alert_sent` flag prevents repeated alerts; resets when signal resumes
   - Use `esp_println::println!` for all output (not `defmt` — espflash monitor cannot decode defmt frames without the ELF)
4. Create a `CLAUDE.md` at the project root documenting: hardware wiring, active-high/low behaviour, key constants, toolchain setup, and build/flash commands

Remind the user that if the sensor polarity is unknown, the diagnostic approach is:
- Add a `println!` on both `is_high()` and `is_low()` branches
- Flash and physically trigger the sensor
- Whichever branch logs confirms the active level
- 5 ms poll interval is critical — 100 ms will miss brief pulses from mechanical sensors like the SW-420

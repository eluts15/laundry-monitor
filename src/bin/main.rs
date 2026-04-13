#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Pull};
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_println::println;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();


/// How long the sensor must be still before declaring the cycle complete.
const IDLE_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// How often to poll the sensor (5 ms).
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // SW-420 is active-high: D0 goes HIGH when vibrating, LOW when still.
    // Pull-down ensures a stable LOW reading when the sensor is quiet.
    let sensor = Input::new(
        peripherals.GPIO25,
        InputConfig::default().with_pull(Pull::Down),
    );

    println!("[DEBUG] Laundry monitor started — waiting for vibrations...");

    // Tracks the last moment a vibration was detected.
    // `None` means the washer hasn't started yet.
    let mut last_vibration: Option<Instant> = None;

    // Prevents the alert from repeating every poll cycle once it fires.
    let mut alert_sent = false;

    // State tracking for transition logging.
    let mut prev_is_high = false;

    // Heartbeat: log elapsed idle time every 10 seconds.
    let mut last_heartbeat = Instant::now();

    loop {
        let is_high = sensor.is_high();

        // Log on state transitions only.
        if is_high && !prev_is_high {
            println!("[DEBUG] State -> VIBRATING");
        } else if !is_high && prev_is_high {
            println!("[DEBUG] State -> STILL");
        }
        prev_is_high = is_high;

        if is_high {
            last_vibration = Some(Instant::now());
            last_heartbeat = Instant::now();
            if alert_sent {
                println!("[DEBUG] State -> VIBRATING (new cycle after completion)");
            }
            alert_sent = false;
        }

        if let Some(last) = last_vibration {
            let elapsed = last.elapsed();

            if !alert_sent && last_heartbeat.elapsed() >= Duration::from_secs(10) {
                println!(
                    "[DEBUG] State -> IDLE ({} / {} secs without vibration)",
                    elapsed.as_secs(),
                    IDLE_TIMEOUT.as_secs(),
                );
                last_heartbeat = Instant::now();
            }

            if !alert_sent && elapsed >= IDLE_TIMEOUT {
                println!("[DEBUG] Laundry cycle complete — no vibration detected for 2 minutes!");
                alert_sent = true;
            }
        }

        blocking_delay(POLL_INTERVAL);
    }
}

fn blocking_delay(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {}
}

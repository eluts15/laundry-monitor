use blocking_network_stack::Stack;
use esp_hal::time::{Duration, Instant};
use esp_println::println;

use crate::notify::send_ntfy_notification;
use smoltcp::wire::Ipv4Address;

/// Tracks the vibration state and notification lifecycle for a single appliance.
///
/// Each appliance has its own sensor topic, idle timeout, and independent state.
/// Call `poll()` on every iteration of the main loop, passing the current
/// sensor reading and the network stack.
pub struct Appliance {
    /// Human-readable label used in log messages (e.g. "Washer", "Dryer").
    pub name: &'static str,
    /// ntfy topic to POST to when this appliance's cycle completes.
    pub topic: &'static str,
    /// How long the sensor must remain still before the cycle is considered done.
    pub idle_timeout: Duration,

    // -- Internal state -------------------------------------------------------
    /// Timestamp of the most recent HIGH reading from the sensor.
    last_vibration: Option<Instant>,
    /// True once a notification has been sent for the current cycle.
    /// Prevents duplicate alerts and is cleared when a new cycle begins.
    alert_sent: bool,
    /// Sensor state on the previous poll, used to detect and log transitions.
    prev_is_high: bool,
    /// Timestamp used to throttle idle progress log messages.
    last_heartbeat: Instant,
}

impl Appliance {
    pub fn new(name: &'static str, topic: &'static str, idle_timeout: Duration) -> Self {
        Self {
            name,
            topic,
            idle_timeout,
            last_vibration: None,
            alert_sent: false,
            prev_is_high: false,
            last_heartbeat: Instant::now(),
        }
    }

    /// Called on every main loop iteration with the current sensor pin state.
    ///
    /// Handles state transition logging, idle countdown heartbeats, and fires
    /// the ntfy notification once the appliance has been still for `idle_timeout`.
    pub fn poll<D: smoltcp::phy::Device>(
        &mut self,
        is_high: bool,
        stack: &mut Stack<'_, D>,
        host_ip: Ipv4Address,
        port: u16,
    ) {
        // Log on state transitions only — avoids serial spam at 200 Hz.
        if is_high && !self.prev_is_high {
            println!("[DEBUG] {} -> VIBRATING", self.name);
        } else if !is_high && self.prev_is_high {
            println!("[DEBUG] {} -> STILL", self.name);
        }
        self.prev_is_high = is_high;

        if is_high {
            self.last_vibration = Some(Instant::now());
            self.last_heartbeat = Instant::now();
            if self.alert_sent {
                println!(
                    "[DEBUG] {} new cycle detected after previous completion.",
                    self.name
                );
            }
            // Clear alert_sent so a new cycle can trigger a fresh notification.
            self.alert_sent = false;
        }

        if let Some(last) = self.last_vibration {
            let elapsed = last.elapsed();

            // Emit a progress message every 10 s during the idle countdown.
            if !self.alert_sent && self.last_heartbeat.elapsed() >= Duration::from_secs(10) {
                println!(
                    "[DEBUG] {} IDLE ({}/{} s without vibration)",
                    self.name,
                    elapsed.as_secs(),
                    self.idle_timeout.as_secs(),
                );
                self.last_heartbeat = Instant::now();
            }

            if !self.alert_sent && elapsed >= self.idle_timeout {
                println!(
                    "[INFO] {} cycle complete — sending ntfy notification...",
                    self.name
                );
                send_ntfy_notification(stack, host_ip, port, self.topic);
                self.alert_sent = true;
            }
        }
    }
}

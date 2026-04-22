#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use blocking_network_stack::Stack;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Pull};
use esp_hal::main;
use esp_hal::rng::Rng;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::wifi::{ClientConfig, ModeConfig};
use laundry_monitor::notify::send_ntfy_notification;
use laundry_monitor::utils::{blocking_delay, octet, parse_u16};
use laundry_monitor::wifi_adapter::WifiAdapter;
use smoltcp::iface::{SocketSet, SocketStorage};
use smoltcp::wire::Ipv4Address;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[PANIC] {}", info);
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

// WiFi credentials
const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");

/// Local IP of the machine running the ntfy Docker container.
const HOST_IP: Ipv4Address = Ipv4Address::new(
    octet(env!("HOST_IP_0")),
    octet(env!("HOST_IP_1")),
    octet(env!("HOST_IP_2")),
    octet(env!("HOST_IP_3")),
);

const NFTY_PORT: u16 = parse_u16(env!("NFTY_PORT"));
const NFTY_TOPIC: &str = env!("NFTY_TOPIC");

/// How long the sensor must be still before declaring the cycle complete.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// How often to poll the sensor (5 ms catches brief SW-420 pulses).
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Heap required by esp-radio/esp-rtos internals (~72 KB is a safe floor for ESP32).
    esp_alloc::heap_allocator!(size: 72 * 1024);

    // ── WiFi / radio init ───────────────────────────────────────────────────
    // esp-rtos must be started before esp_radio::init().
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let radio_init = esp_radio::init().expect("Radio init failed");

    let (mut wifi_ctrl, interfaces) =
        esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
            .expect("WiFi new failed");

    wifi_ctrl
        .set_config(&ModeConfig::Client(
            ClientConfig::default()
                .with_ssid(WIFI_SSID.into())
                .with_password(WIFI_PASSWORD.into()),
        ))
        .expect("WiFi set_config failed");

    wifi_ctrl.start().expect("WiFi start failed");
    wifi_ctrl.connect().expect("WiFi connect failed");

    println!("[INFO] Connecting to '{}'...", WIFI_SSID);
    loop {
        match wifi_ctrl.is_connected() {
            Ok(true) => break,
            Ok(false) => {}
            Err(e) => panic!("WiFi error: {:?}", e),
        }
    }
    println!("[INFO] WiFi connected.");

    // ── Network stack ───────────────────────────────────────────────────────
    let mut adapter = WifiAdapter(interfaces.sta);

    let now = || Instant::now().duration_since_epoch().as_millis();
    let rng = Rng::new();

    let mut socket_storage: [SocketStorage<'_>; 3] = Default::default();
    let mut socket_set = SocketSet::new(&mut socket_storage[..]);
    // blocking-network-stack discovers the DHCP socket by scanning the set at
    // Stack::new() time. Without a pre-added socket, dhcp_socket_handle stays
    // None and poll_dhcp() is a no-op — causing the DHCP loop to hang forever.
    socket_set.add(smoltcp::socket::dhcpv4::Socket::new());

    let iface = {
        let mac = adapter.mac_address();
        let hw_addr = smoltcp::wire::EthernetAddress::from_bytes(&mac);
        let cfg = smoltcp::iface::Config::new(hw_addr.into());
        smoltcp::iface::Interface::new(cfg, &mut adapter, smoltcp::time::Instant::ZERO)
    };

    let mut stack = Stack::new(iface, adapter, socket_set, now, rng.random());

    // Wait for a DHCP lease.
    println!("[INFO] Waiting for DHCP...");
    loop {
        stack.work();
        if stack.is_iface_up() {
            println!("[INFO] IP: {:?}", stack.get_ip_info());
            break;
        }
    }
    println!("[INFO] Network ready.");

    // ── Sensor GPIO ─────────────────────────────────────────────────────────
    // SW-420 is active-high: D0 goes HIGH when vibrating, LOW when still.
    // Pull-down ensures a stable LOW reading when the sensor is quiet.
    let sensor = Input::new(
        peripherals.GPIO25,
        InputConfig::default().with_pull(Pull::Down),
    );

    println!("[INFO] Laundry monitor started — waiting for vibrations...");

    let mut last_vibration: Option<Instant> = None;
    let mut alert_sent = false;
    let mut prev_is_high = false;
    let mut last_heartbeat = Instant::now();

    // ── Main poll loop ───────────────────────────────────────────────────────
    loop {
        // Keep the network stack alive between sensor polls.
        stack.work();

        let is_high = sensor.is_high();

        // Log only on state transitions.
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
                println!("[DEBUG] New cycle detected after previous completion.");
            }
            alert_sent = false;
        }

        if let Some(last) = last_vibration {
            let elapsed = last.elapsed();

            if !alert_sent && last_heartbeat.elapsed() >= Duration::from_secs(10) {
                println!(
                    "[DEBUG] IDLE ({}/{} s without vibration)",
                    elapsed.as_secs(),
                    IDLE_TIMEOUT.as_secs(),
                );
                last_heartbeat = Instant::now();
            }

            if !alert_sent && elapsed >= IDLE_TIMEOUT {
                println!("[INFO] Cycle complete — sending ntfy notification...");
                send_ntfy_notification(&mut stack, HOST_IP, NFTY_PORT, NFTY_TOPIC);
                alert_sent = true;
            }
        }

        blocking_delay(POLL_INTERVAL);
    }
}


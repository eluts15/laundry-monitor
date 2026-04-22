#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::fmt::Write;
use core::task::{Context, RawWaker, RawWakerVTable, Waker};

use blocking_network_stack::Stack;
use embassy_net_driver::Driver as EmbDriver;
use embedded_io::{Read, Write as IoWrite};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Pull};
use esp_hal::main;
use esp_hal::rng::Rng;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::wifi::{ClientConfig, ModeConfig};
use smoltcp::iface::{SocketSet, SocketStorage};
use smoltcp::wire::Ipv4Address;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

// WiFi credentials
const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");

const fn octet(s: &str) -> u8 {
    match s.as_bytes() {
        [a] => *a - b'0',
        [a, b] => (*a - b'0') * 10 + (*b - b'0'),
        [a, b, c] => (*a - b'0') * 100 + (*b - b'0') * 10 + (*c - b'0'),
        _ => panic!("invalid octet"),
    }
}

/// Local IP of the machine running the ntfy Docker container.
const HOST_IP: Ipv4Address = Ipv4Address::new(
    octet(env!("HOST_IP_0")),
    octet(env!("HOST_IP_1")),
    octet(env!("HOST_IP_2")),
    octet(env!("HOST_IP_3")),
);

const fn parse_u16(s: &str) -> u16 {
    let s = s.as_bytes();
    let mut val: u16 = 0;
    let mut i = 0;
    while i < s.len() {
        val = val * 10 + (s[i] - b'0') as u16;
        i += 1;
    }
    val
}

const NFTY_PORT: u16 = parse_u16(env!("NFTY_PORT"));
const NFTY_TOPIC: &str = env!("NFTY_TOPIC");
/// How long the sensor must be still before declaring the cycle complete.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// How often to poll the sensor (5 ms catches brief SW-420 pulses).
const POLL_INTERVAL: Duration = Duration::from_millis(5);

// ── WifiDevice → smoltcp::phy::Device adapter ──────────────────────────────
//
// esp-radio 0.17's WifiDevice implements embassy_net_driver::Driver but not
// smoltcp::phy::Device directly. This thin adapter bridges the two traits so
// we can keep the blocking-network-stack architecture.
//
// A noop waker is used because we poll in a busy-wait loop — we never need
// the driver to wake a future.

fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(core::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
}

struct WifiAdapter<'d>(esp_radio::wifi::WifiDevice<'d>);

impl<'d> WifiAdapter<'d> {
    fn mac_address(&self) -> [u8; 6] {
        match EmbDriver::hardware_address(&self.0) {
            embassy_net_driver::HardwareAddress::Ethernet(addr) => addr,
            _ => [0u8; 6],
        }
    }
}

struct SmoltcpRxToken<T: embassy_net_driver::RxToken>(T);
struct SmoltcpTxToken<T: embassy_net_driver::TxToken>(T);

impl<T: embassy_net_driver::RxToken> smoltcp::phy::RxToken for SmoltcpRxToken<T> {
    // smoltcp 0.12 uses &[u8] (shared); embassy uses &mut [u8] (mutable).
    // &mut [u8] coerces to &[u8] so passing buf directly to f works.
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        let mut ret: Option<R> = None;
        self.0.consume(|buf| {
            ret = Some(f(buf));
        });
        ret.unwrap() // consume always calls the closure exactly once
    }
}

impl<T: embassy_net_driver::TxToken> smoltcp::phy::TxToken for SmoltcpTxToken<T> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut ret: Option<R> = None;
        self.0.consume(len, |buf| {
            ret = Some(f(buf));
        });
        ret.unwrap()
    }
}

impl<'d> smoltcp::phy::Device for WifiAdapter<'d> {
    type RxToken<'a>
        = SmoltcpRxToken<<esp_radio::wifi::WifiDevice<'d> as EmbDriver>::RxToken<'a>>
    where
        Self: 'a;
    type TxToken<'a>
        = SmoltcpTxToken<<esp_radio::wifi::WifiDevice<'d> as EmbDriver>::TxToken<'a>>
    where
        Self: 'a;

    fn receive(
        &mut self,
        _: smoltcp::time::Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        EmbDriver::receive(&mut self.0, &mut cx)
            .map(|(rx, tx)| (SmoltcpRxToken(rx), SmoltcpTxToken(tx)))
    }

    fn transmit(&mut self, _: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        EmbDriver::transmit(&mut self.0, &mut cx).map(SmoltcpTxToken)
    }

    fn capabilities(&self) -> smoltcp::phy::DeviceCapabilities {
        let mut caps = smoltcp::phy::DeviceCapabilities::default();
        caps.medium = smoltcp::phy::Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps.max_burst_size = Some(1);
        caps
    }
}

// Static socket buffers — satisfy Stack's lifetime requirements.
// SAFETY: only accessed from a single task in the main loop.
static mut SOCK_RX: [u8; 1536] = [0u8; 1536];
static mut SOCK_TX: [u8; 1536] = [0u8; 1536];

// ── Entry point ─────────────────────────────────────────────────────────────

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
    // Wrap the STA device in the smoltcp adapter.
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
                send_ntfy_notification(&mut stack);
                alert_sent = true;
            }
        }

        blocking_delay(POLL_INTERVAL);
    }
}

/// POSTs a notification to the local ntfy instance.
fn send_ntfy_notification<D: smoltcp::phy::Device>(stack: &mut Stack<'_, D>) {
    let body = "Laundry cycle complete!";

    // SAFETY: called from a single task; buffers are not aliased.
    let (rx, tx) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(SOCK_RX),
            &mut *core::ptr::addr_of_mut!(SOCK_TX),
        )
    };
    let mut socket = stack.get_socket(rx, tx);

    if let Err(e) = socket.open(smoltcp::wire::IpAddress::Ipv4(HOST_IP), NFTY_PORT) {
        println!("[ERROR] TCP open failed: {:?}", e);
        return;
    }

    // Build a minimal HTTP/1.0 POST.
    // HTTP/1.0: server closes the connection after responding, so we
    // don't need to parse headers to detect end-of-response.
    let mut request: heapless::String<512> = heapless::String::new();
    write!(
        request,
        "POST /{} HTTP/1.0\r\nHost: {}:{}\r\nContent-Type: text/plain\r\nTitle: Laundry Done\r\nContent-Length: {}\r\n\r\n{}",
        NFTY_TOPIC, HOST_IP, NFTY_PORT, body.len(), body
    )
    .unwrap();

    if let Err(e) = IoWrite::write_all(&mut socket, request.as_bytes()) {
        println!("[ERROR] TCP write failed: {:?}", e);
        socket.disconnect();
        return;
    }
    socket.flush().ok();

    // Drain the response (we only care that the request was accepted).
    let deadline = Instant::now();
    let mut buf = [0u8; 256];
    loop {
        match socket.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if deadline.elapsed() > Duration::from_secs(5) {
            break;
        }
    }

    socket.disconnect();
    println!("[INFO] ntfy notification sent (topic: '{}').", NFTY_TOPIC);
}

fn blocking_delay(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {}
}

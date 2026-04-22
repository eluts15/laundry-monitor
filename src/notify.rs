use core::fmt::Write;

use blocking_network_stack::Stack;
use embedded_io::{Read, Write as IoWrite};
use esp_hal::time::{Duration, Instant};
use esp_println::println;
use smoltcp::wire::Ipv4Address;

// Static socket buffers — satisfy Stack's lifetime requirements.
// SAFETY: only accessed from a single task in the main loop.
static mut SOCK_RX: [u8; 1536] = [0u8; 1536];
static mut SOCK_TX: [u8; 1536] = [0u8; 1536];

/// POSTs a notification to the local ntfy instance.
pub fn send_ntfy_notification<D: smoltcp::phy::Device>(
    stack: &mut Stack<'_, D>,
    host_ip: Ipv4Address,
    port: u16,
    topic: &str,
) {
    let body = "Laundry cycle complete!";

    // SAFETY: called from a single task; buffers are not aliased.
    let (rx, tx) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(SOCK_RX),
            &mut *core::ptr::addr_of_mut!(SOCK_TX),
        )
    };
    let mut socket = stack.get_socket(rx, tx);

    if let Err(e) = socket.open(smoltcp::wire::IpAddress::Ipv4(host_ip), port) {
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
        topic, host_ip, port, body.len(), body
    )
    .unwrap();

    if let Err(e) = IoWrite::write_all(&mut socket, request.as_bytes()) {
        println!("[ERROR] TCP write failed: {:?}", e);
        socket.disconnect();
        return;
    }
    socket.flush().ok();

    // Drain the response (we only care that the request was accepted).
    // `deadline` is set before the loop to cap the total response budget to 5s.
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
    println!("[INFO] ntfy notification sent (topic: '{}').", topic);
}

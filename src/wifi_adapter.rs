use core::task::Context;

use embassy_net_driver::Driver as EmbDriver;

use crate::utils::noop_waker;

// ── WifiDevice → smoltcp::phy::Device adapter ──────────────────────────────
//
// esp-radio 0.17's WifiDevice implements embassy_net_driver::Driver but not
// smoltcp::phy::Device directly. This thin adapter bridges the two traits so
// we can keep the blocking-network-stack architecture.
//
// A noop waker is used because we poll in a busy-wait loop — we never need
// the driver to wake a future.

pub struct WifiAdapter<'d>(pub esp_radio::wifi::WifiDevice<'d>);

impl<'d> WifiAdapter<'d> {
    pub fn mac_address(&self) -> [u8; 6] {
        match EmbDriver::hardware_address(&self.0) {
            embassy_net_driver::HardwareAddress::Ethernet(addr) => addr,
            _ => [0u8; 6],
        }
    }
}

pub struct SmoltcpRxToken<T: embassy_net_driver::RxToken>(pub T);
pub struct SmoltcpTxToken<T: embassy_net_driver::TxToken>(pub T);

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

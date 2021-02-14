use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::{Arc, RwLock};
use std::vec::Vec;

use smoltcp::phy;
use smoltcp::phy::{Device, DeviceCapabilities};
use smoltcp::time::Instant;
use smoltcp::Result;

use tap_interface_sys;

use SMOLTCP_ETHERNET_HEADER;

/// A virtual Ethernet interface.
#[derive(Debug)]
pub struct TapInterface {
    lower: Arc<RwLock<tap_interface_sys::TapInterfaceDesc>>,
    mtu: usize,
    reduce_mtu_by: Option<usize>,
}

impl AsRawFd for TapInterface {
    fn as_raw_fd(&self) -> RawFd {
        self.lower.read().unwrap().as_raw_fd()
    }
}

impl TapInterface {
    /// Attaches to a TAP interface called `name`, or creates it if it does not exist.
    ///
    /// If `name` is a persistent interface configured with UID of the current user,
    /// no special privileges are needed. Otherwise, this requires superuser privileges
    /// or a corresponding capability set on the executable.
    pub fn new(name: &str, reduce_mtu_by: Option<usize>) -> io::Result<TapInterface> {
        let mut lower = tap_interface_sys::TapInterfaceDesc::new(name)?;
        lower.attach_interface()?;
        let mtu = lower.interface_mtu()?;
        Ok(TapInterface {
            lower: Arc::new(RwLock::new(lower)),
            mtu: mtu + SMOLTCP_ETHERNET_HEADER,
            reduce_mtu_by: reduce_mtu_by,
        })
    }

    /// Attaches to a MACVTAP interface called `name`.
    ///
    /// If the character device for `name` is owned by the current user,
    /// no special privileges are needed. Otherwise, this requires superuser privileges
    /// or a corresponding capability set on the executable.
    /// `ip link add link wlp3s0 name macvtap0 type macvtap mode bridge|passthru|private|vepa|source`
    /// `sudo ip link set macvtap0 address 76:02:3f:d0:af:f0 up`
    /// The attached MAC address must also be used in smoltcp
    /// (for passthru it is the same as the underlying device).
    pub fn new_macvtap(name: &str, reduce_mtu_by: Option<usize>) -> io::Result<TapInterface> {
        let mut lower = tap_interface_sys::TapInterfaceDesc::new_macvtap(name)?;
        lower.attach_interface()?;
        let mtu = lower.interface_mtu()?;
        Ok(TapInterface {
            lower: Arc::new(RwLock::new(lower)),
            mtu: mtu + SMOLTCP_ETHERNET_HEADER,
            reduce_mtu_by: reduce_mtu_by,
        })
    }
}

impl<'a> Device<'a> for TapInterface {
    type RxToken = RxToken;
    type TxToken = TxToken;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = self.mtu - self.reduce_mtu_by.unwrap_or(0);
        caps
    }

    fn receive(&'a mut self) -> Option<(Self::RxToken, Self::TxToken)> {
        let mut lower = self.lower.write().unwrap();
        let mut buffer = vec![0; self.mtu];
        match lower.recv(&mut buffer[..]) {
            Ok(size) => {
                buffer.resize(size, 0);
                let rx = RxToken { buffer };
                let tx = TxToken {
                    lower: self.lower.clone(),
                };
                Some((rx, tx))
            }
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => None,
            Err(err) => panic!("{}", err),
        }
    }

    fn transmit(&'a mut self) -> Option<Self::TxToken> {
        Some(TxToken {
            lower: self.lower.clone(),
        })
    }
}

#[doc(hidden)]
pub struct RxToken {
    buffer: Vec<u8>,
}

impl phy::RxToken for RxToken {
    fn consume<R, F>(self, _timestamp: Instant, f: F) -> Result<R>
    where
        F: FnOnce(&mut [u8]) -> Result<R>,
    {
        let mut buffer = self.buffer.clone();
        f(&mut buffer[..])
    }
}

#[doc(hidden)]
pub struct TxToken {
    lower: Arc<RwLock<tap_interface_sys::TapInterfaceDesc>>,
}

impl phy::TxToken for TxToken {
    fn consume<R, F>(self, _timestamp: Instant, len: usize, f: F) -> Result<R>
    where
        F: FnOnce(&mut [u8]) -> Result<R>,
    {
        let mut lower = self.lower.write().unwrap();
        let mut buffer = vec![0; len];
        let result = f(&mut buffer);
        lower.send(&buffer[..]).unwrap();
        result
    }
}

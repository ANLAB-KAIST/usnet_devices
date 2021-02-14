use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixDatagram;
use std::sync::{Arc, RwLock};
use std::vec::Vec;

use smoltcp::phy;
use smoltcp::phy::{Device, DeviceCapabilities};
use smoltcp::time::Instant;
use smoltcp::{Error, Result};
use uds;

use SMOLTCP_ETHERNET_HEADER;

/// A socket that captures or transmits the complete frame.
#[derive(Debug)]
pub struct UnixDomainSocket {
    lower: Arc<RwLock<uds::UnixDomainSocketDesc>>,
    mtu: usize,
    reduce_mtu_by: Option<usize>,
}

impl AsRawFd for UnixDomainSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.lower.read().unwrap().as_raw_fd()
    }
}

impl UnixDomainSocket {
    pub fn new_from_unix_datagram(
        from: UnixDatagram,
        parent: &str,
        reduce_mtu_by: Option<usize>,
    ) -> io::Result<UnixDomainSocket> {
        let mut lower = uds::UnixDomainSocketDesc::new_from_unix_datagram(from, parent)?;
        let mtu = lower.interface_mtu()?;
        Ok(UnixDomainSocket {
            lower: Arc::new(RwLock::new(lower)),
            mtu: mtu + SMOLTCP_ETHERNET_HEADER,
            reduce_mtu_by: reduce_mtu_by,
        })
    }
}

impl<'a> Device<'a> for UnixDomainSocket {
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
    fn consume<R, F: FnOnce(&mut [u8]) -> Result<R>>(self, _timestamp: Instant, f: F) -> Result<R> {
        let mut buffer = self.buffer.clone();
        f(&mut buffer[..])
    }
}

#[doc(hidden)]
pub struct TxToken {
    lower: Arc<RwLock<uds::UnixDomainSocketDesc>>,
}

impl phy::TxToken for TxToken {
    fn consume<R, F: FnOnce(&mut [u8]) -> Result<R>>(
        self,
        _timestamp: Instant,
        len: usize,
        f: F,
    ) -> Result<R> {
        let mut lower = self.lower.write().unwrap();
        let mut buffer = vec![0; len];
        let result = f(&mut buffer);
        match lower.send(&buffer[..]) {
            Ok(_) => result,
            Err(ref err) => {
                if err.kind() == io::ErrorKind::WouldBlock {
                    Err(Error::Exhausted)
                } else {
                    Err(Error::Unaddressable)
                }
            }
        }
    }
}

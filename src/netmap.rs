use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::{Arc, RwLock};

use nm;
use smoltcp::phy;
use smoltcp::phy::{Device, DeviceCapabilities};
use smoltcp::time::Instant;
use smoltcp::Result;

use SMOLTCP_ETHERNET_HEADER;

pub use nm::nmreq;

/// Netmap provies a virtual Ethernet interface.
/// smoltcp compatible Netmap (w/ rx sync ioctl, implicit batched tx, parent mtu, no recv_ready, no explicit issue_tx_sync, no zc_forward)
#[derive(Debug)]
pub struct Netmap {
    lower: Arc<RwLock<nm::NetmapDesc>>,
    mtu: usize,
    reduce_mtu_by: Option<usize>,
}

impl AsRawFd for Netmap {
    fn as_raw_fd(&self) -> RawFd {
        self.lower.read().unwrap().as_raw_fd()
    }
}

impl Netmap {
    /// Attaches to a Netmap interface specified by `name` (see Netmap
    /// documentation, e.g. "netmap:eth0").
    ///
    /// Since the interface may be a pipe or vale port etc. the `parent` name
    /// refers to the underlying system interface for MTU discovery.
    /// If `uses_wait` is set, then `wait` needs to be used in order to receive
    /// packets because it calls `select`. If `wait` is not used, then a value
    /// of `false` for `uses_wait` will cause issueing RXSYNC ioctls on receival.
    pub fn new(
        name: &str,
        parent: &str,
        uses_wait: bool,
        reduce_mtu_by: Option<usize>,
    ) -> io::Result<Netmap> {
        let mut lower = nm::NetmapDesc::new(name, parent, uses_wait)?;
        let mtu = lower.interface_mtu()?;
        Ok(Netmap {
            lower: Arc::new(RwLock::new(lower)),
            mtu: mtu + SMOLTCP_ETHERNET_HEADER,
            reduce_mtu_by: reduce_mtu_by,
        })
    }

    /// Attaches to a Netmap interface opened by another process which shared
    /// the file descriptor via Unix Domain Socket sendmsg IPC.
    ///
    /// Since the interface may be a pipe or vale port etc. the `parent` name
    /// refers to the underlying system interface for MTU discovery.
    /// If `uses_wait` is set, then `wait` needs to be used in order to receive
    /// packets because it calls `select`. If `wait` is not used, then a value
    /// of `false` for `uses_wait` will cause issueing RXSYNC ioctls on receival.
    pub fn new_from_shared_fd(
        fd: RawFd,
        req: nmreq,
        parent: &str,
        uses_wait: bool,
        reduce_mtu_by: Option<usize>,
    ) -> io::Result<Netmap> {
        let mut lower = nm::NetmapDesc::new_from_shared_fd(fd, req, parent, uses_wait)?;
        let mtu = lower.interface_mtu()?;
        Ok(Netmap {
            lower: Arc::new(RwLock::new(lower)),
            mtu: mtu + SMOLTCP_ETHERNET_HEADER,
            reduce_mtu_by: reduce_mtu_by,
        })
    }

    pub fn tx_flush(&mut self) -> Result<()> {
        let mut lower = self.lower.write().unwrap();
        lower.tx_flush()
    }

    pub fn set_uses_wait(&mut self, uses_wait: bool) {
        let mut lower = self.lower.write().unwrap();
        lower.set_uses_wait(uses_wait);
    }

    pub fn get_uses_wait(&self) -> bool {
        let lower = self.lower.read().unwrap();
        lower.get_uses_wait()
    }

    pub fn get_nmreq(&self) -> nmreq {
        let lower = self.lower.read().unwrap();
        lower.get_nmreq()
    }

    pub fn zc_forward(&mut self, from: &mut Netmap) -> Result<()> {
        let mut lower = self.lower.write().unwrap();
        let mut from_lower = from.lower.write().unwrap();
        lower.zc_forward(&mut from_lower)
    }
}

impl<'a> Device<'a> for Netmap {
    type RxToken = RxToken;
    type TxToken = TxToken;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = self.mtu - self.reduce_mtu_by.unwrap_or(0);;
        caps
    }

    fn receive(&'a mut self) -> Option<(Self::RxToken, Self::TxToken)> {
        let mut lower = self.lower.write().unwrap();
        match lower.recv() {
            Ok(buf) => {
                let rx = RxToken { read_buffer: buf };
                // We could test if TX is available, but this would block RX…,
                // and the waiting logic is also only focused on RX
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
        let r = self.lower.read().unwrap().send_ready();
        match r {
            Ok(_) => Some(TxToken {
                lower: self.lower.clone(),
            }),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                // workaround for https://github.com/luigirizzo/netmap/issues/457
                let _ = self.tx_flush();
                Some(TxToken {
                    lower: self.lower.clone(),
                })
                // done
                // None
            }
            Err(err) => panic!("{}", err),
        }
    }
}

#[doc(hidden)]
pub struct RxToken {
    read_buffer: &'static [u8], // safe usage only before next receive
}

impl<'a> phy::RxToken for RxToken {
    fn consume<R, F>(self, _timestamp: Instant, f: F) -> Result<R>
    where
        F: FnOnce(&[u8]) -> Result<R>,
    {
        f(self.read_buffer)
    }
}

#[doc(hidden)]
pub struct TxToken {
    lower: Arc<RwLock<nm::NetmapDesc>>,
}

impl phy::TxToken for TxToken {
    fn consume<R, F>(self, _timestamp: Instant, len: usize, f: F) -> Result<R>
    where
        F: FnOnce(&mut [u8]) -> Result<R>,
    {
        let mut lower = self.lower.write().unwrap();
        lower.send(len, f)
    }
}

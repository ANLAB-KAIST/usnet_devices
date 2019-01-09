use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixDatagram;

use super::{ifreq, ifreq_for, ifreq_ioctl, SIOCGIFMTU};

#[derive(Debug)]
pub struct UnixDomainSocketDesc {
    lower: UnixDatagram,
    ifreq: ifreq,
}

impl AsRawFd for UnixDomainSocketDesc {
    fn as_raw_fd(&self) -> RawFd {
        self.lower.as_raw_fd()
    }
}

impl UnixDomainSocketDesc {
    pub fn new_from_unix_datagram(
        from: UnixDatagram,
        parent: &str,
    ) -> io::Result<UnixDomainSocketDesc> {
        from.set_nonblocking(true)?;

        Ok(UnixDomainSocketDesc {
            lower: from,
            ifreq: ifreq_for(parent),
        })
    }

    pub fn interface_mtu(&mut self) -> io::Result<usize> {
        ifreq_ioctl(self.lower.as_raw_fd(), &mut self.ifreq, SIOCGIFMTU).map(|mtu| mtu as usize)
    }

    pub fn recv(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.lower.recv(buffer)
    }

    pub fn send(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.lower.send(buffer)
    }
}

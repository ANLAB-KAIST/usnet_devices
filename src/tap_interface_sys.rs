use libc;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::string::{String, ToString};

use super::{
    ifreq, ifreq_for, ifreq_ioctl, IFF_NO_PI, IFF_TAP, SIOCGIFINDEX, SIOCGIFMTU, TUNSETIFF,
};

#[derive(Debug)]
pub struct TapInterfaceDesc {
    lower: libc::c_int,
    ifreq: ifreq,
}

impl AsRawFd for TapInterfaceDesc {
    fn as_raw_fd(&self) -> RawFd {
        self.lower
    }
}

impl TapInterfaceDesc {
    pub fn new(name: &str) -> io::Result<TapInterfaceDesc> {
        let lower = unsafe {
            let lower = libc::open(
                "/dev/net/tun\0".as_ptr() as *const libc::c_char,
                libc::O_RDWR | libc::O_NONBLOCK,
            );
            if lower == -1 {
                return Err(io::Error::last_os_error());
            }
            lower
        };

        Ok(TapInterfaceDesc {
            lower: lower,
            ifreq: ifreq_for(name),
        })
    }

    pub fn new_macvtap(name: &str) -> io::Result<TapInterfaceDesc> {
        let mut ifreq = ifreq_for(name);
        let ifindex = {
            let socketfd = unsafe {
                let socketfd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_IP);
                if socketfd == -1 {
                    return Err(io::Error::last_os_error());
                }
                socketfd
            };

            let ifindex =
                ifreq_ioctl(socketfd, &mut ifreq, SIOCGIFINDEX).map(|ifindex| ifindex as usize);

            unsafe {
                libc::close(socketfd);
            }

            ifindex
        }?;

        let lower = unsafe {
            let lower = libc::open(
                (String::from("/dev/tap") + &ifindex.to_string() + "\0").as_ptr()
                    as *const libc::c_char,
                libc::O_RDWR | libc::O_NONBLOCK,
            );
            if lower == -1 {
                return Err(io::Error::last_os_error());
            }
            lower
        };

        Ok(TapInterfaceDesc {
            lower: lower,
            ifreq: ifreq,
        })
    }

    pub fn attach_interface(&mut self) -> io::Result<()> {
        self.ifreq.ifr_data = IFF_TAP | IFF_NO_PI;
        ifreq_ioctl(self.lower, &mut self.ifreq, TUNSETIFF).map(|_| ())
    }

    pub fn interface_mtu(&mut self) -> io::Result<usize> {
        let lower = unsafe {
            let lower = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_IP);
            if lower == -1 {
                return Err(io::Error::last_os_error());
            }
            lower
        };

        let mtu = ifreq_ioctl(lower, &mut self.ifreq, SIOCGIFMTU).map(|mtu| mtu as usize);

        unsafe {
            libc::close(lower);
        }

        mtu
    }

    pub fn recv(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        unsafe {
            let len = libc::read(
                self.lower,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
            );
            if len == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(len as usize)
        }
    }

    pub fn send(&mut self, buffer: &[u8]) -> io::Result<usize> {
        unsafe {
            let len = libc::write(
                self.lower,
                buffer.as_ptr() as *const libc::c_void,
                buffer.len(),
            );
            if len == -1 {
                Err(io::Error::last_os_error()).unwrap()
            }
            Ok(len as usize)
        }
    }
}

impl Drop for TapInterfaceDesc {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.lower);
        }
    }
}

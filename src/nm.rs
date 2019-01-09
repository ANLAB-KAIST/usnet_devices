use std::fs;
use std::io;
use std::mem;
use std::os::unix::io::{AsRawFd, RawFd};
use std::ptr;
use std::slice;
use std::string::ToString;

use smoltcp::{Error, Result};

extern crate netmap_sys;

pub use self::netmap_sys::netmap::nmreq;
use self::netmap_sys::netmap::{
    netmap_slot, nm_ring_empty, NETMAP_RING_MASK, NIOCRXSYNC, NIOCTXSYNC, NR_REG_ALL_NIC,
    NR_REG_MASK, NR_REG_NIC_SW, NR_REG_ONE_NIC, NR_REG_SW, NS_BUF_CHANGED,
};
use self::netmap_sys::netmap_user::{
    nm_close, nm_desc, nm_open, nm_ring_next, NETMAP_BUF, NETMAP_FD, NETMAP_RXRING, NETMAP_TXRING,
};

use super::{ifreq, ifreq_for, ifreq_ioctl, SIOCGIFMTU};
use libc;

use libc::c_int;

extern "C" {
    pub fn nm_mmap(nm_desc: *mut nm_desc, parent: *const nm_desc) -> c_int;
}

#[derive(Debug)]
pub struct NetmapDesc {
    nm_desc: *mut nm_desc,
    zc_rx_slot: Option<*mut netmap_slot>,
    boxed: bool,
    buf_size: u16,
    ifreq: ifreq,
    uses_wait: bool,
}

unsafe impl Send for NetmapDesc {}
unsafe impl Sync for NetmapDesc {}

impl AsRawFd for NetmapDesc {
    fn as_raw_fd(&self) -> RawFd {
        unsafe { NETMAP_FD(self.nm_desc) }
    }
}

impl NetmapDesc {
    pub fn new(name: &str, parent: &str, uses_wait: bool) -> io::Result<NetmapDesc> {
        let ifname = name.to_string() + "\0";
        let nm_desc = unsafe {
            nm_open(
                ifname.as_ptr() as *const libc::c_char,
                ptr::null(),
                0,
                ptr::null(),
            )
        };

        if nm_desc.is_null() {
            Err(io::Error::last_os_error())
        } else {
            let buf_size = fs::read_to_string("/sys/module/netmap/parameters/buf_size")?
                .trim_right()
                .parse()
                .unwrap();

            Ok(NetmapDesc {
                nm_desc: nm_desc,
                zc_rx_slot: None,
                boxed: false,
                buf_size: buf_size,
                ifreq: ifreq_for(parent),
                uses_wait: uses_wait,
            })
        }
    }

    pub fn new_from_shared_fd(
        fd: RawFd,
        req: nmreq,
        parent: &str,
        uses_wait: bool,
    ) -> io::Result<NetmapDesc> {
        let nmd_box: Box<nm_desc> = Box::new(unsafe { mem::zeroed() });
        let des: &'static mut nm_desc = Box::leak(nmd_box);
        des.self_ = des;
        des.fd = fd;
        des.req = req;
        match req.nr_flags & NR_REG_MASK as u32 {
            NR_REG_SW => {
                // host stack
                des.last_tx_ring = des.req.nr_tx_rings;
                des.first_tx_ring = des.last_tx_ring;
                des.last_rx_ring = des.req.nr_rx_rings;
                des.first_rx_ring = des.last_rx_ring;
            }
            NR_REG_ALL_NIC => {
                // only nic
                des.first_tx_ring = 0;
                des.first_rx_ring = 0;
                des.last_tx_ring = des.req.nr_tx_rings - 1;
                des.last_rx_ring = des.req.nr_rx_rings - 1;
            }
            NR_REG_NIC_SW => {
                des.first_tx_ring = 0;
                des.first_rx_ring = 0;
                des.last_tx_ring = des.req.nr_tx_rings;
                des.last_rx_ring = des.req.nr_rx_rings;
            }
            NR_REG_ONE_NIC => {
                let t = des.req.nr_ringid & NETMAP_RING_MASK as u16;
                des.first_tx_ring = t;
                des.last_tx_ring = t;
                des.first_rx_ring = t;
                des.last_rx_ring = t;
            }
            _ => {
                // pipes
                des.first_tx_ring = 0;
                des.last_tx_ring = 0;
                des.first_rx_ring = 0;
                des.last_rx_ring = 0;
            }
        }

        if unsafe { nm_mmap(des, ptr::null()) } != 0 {
            Err(io::Error::last_os_error())
        } else {
            let buf_size = fs::read_to_string("/sys/module/netmap/parameters/buf_size")?
                .trim_right()
                .parse()
                .unwrap();

            Ok(NetmapDesc {
                nm_desc: des,
                zc_rx_slot: None,
                boxed: true,
                buf_size: buf_size,
                ifreq: ifreq_for(parent),
                uses_wait: uses_wait,
            })
        }
    }

    pub fn tx_flush(&mut self) -> Result<()> {
        let res = unsafe { libc::ioctl(NETMAP_FD(self.nm_desc), NIOCTXSYNC.into()) };
        if res == -1 {
            return Err(Error::Illegal);
        }
        Ok(())
    }

    pub fn set_uses_wait(&mut self, uses_wait: bool) {
        self.uses_wait = uses_wait;
    }

    pub fn get_uses_wait(&self) -> bool {
        self.uses_wait
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

    pub fn recv(&mut self) -> io::Result<(&'static [u8])> {
        unsafe fn find_nextpkt(d: *mut nm_desc) -> Option<(&'static [u8], *mut netmap_slot)> {
            let mut ri = (*d).cur_rx_ring;

            loop {
                /* compute current ring to use */
                let ring = NETMAP_RXRING((*d).nifp, ri as isize);
                if !nm_ring_empty(ring) {
                    let i = (*ring).cur;
                    let slots: *mut netmap_slot = mem::transmute(&mut (*ring).slot);
                    let slot = slots.offset(i as isize);
                    let buf = NETMAP_BUF(ring, (*slot).buf_idx as isize);
                    let slice = slice::from_raw_parts(buf as *const u8, (*slot).len as usize);
                    let next = nm_ring_next(ring, i);
                    (*ring).head = next;
                    (*ring).cur = next;
                    (*d).cur_rx_ring = ri as u16;
                    // read or zero copy forward can only work with this buffer before next syscall
                    return Some((slice, slot));
                }
                ri += 1;
                if ri > (*d).last_rx_ring {
                    ri = (*d).first_rx_ring;
                }
                if ri == (*d).cur_rx_ring {
                    break;
                }
            }
            None /* nothing found */
        }
        if let Some((slice, slot)) = unsafe { find_nextpkt(self.nm_desc) } {
            self.zc_rx_slot = Some(slot);
            Ok(slice)
        } else {
            if self.uses_wait {
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "needs phy_wait()",
                ))
            } else {
                let res = unsafe { libc::ioctl(NETMAP_FD(self.nm_desc), NIOCRXSYNC.into()) };
                if res == -1 {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "rx sync failed"));
                }
                if let Some((slice, slot)) = unsafe { find_nextpkt(self.nm_desc) } {
                    self.zc_rx_slot = Some(slot);
                    Ok(slice)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "next call may have success",
                    ))
                }
            }
        }
    }

    pub fn send_ready(&self) -> io::Result<()> {
        unsafe {
            for i in (*self.nm_desc).first_tx_ring..=(*self.nm_desc).last_tx_ring {
                let ring = NETMAP_TXRING((*self.nm_desc).nifp, i as isize);
                if nm_ring_empty(ring) {
                    continue;
                } else {
                    return Ok(());
                }
            }
            Err(io::Error::new(io::ErrorKind::WouldBlock, "tx ring empty"))
        }
    }

    pub fn send<R, F>(&mut self, packet_size: usize, f: F) -> Result<R>
    where
        F: FnOnce(&mut [u8]) -> Result<R>,
    {
        assert!(packet_size <= self.buf_size as usize);
        if self.send_ready().is_err() {
            self.tx_flush()?; // workaround for https://github.com/luigirizzo/netmap/issues/457
        }

        unsafe {
            for i in (*self.nm_desc).first_tx_ring..=(*self.nm_desc).last_tx_ring {
                let ring = NETMAP_TXRING((*self.nm_desc).nifp, i as isize);
                if nm_ring_empty(ring) {
                    continue;
                } else {
                    let current = (*ring).cur;
                    let slots: *mut netmap_slot = mem::transmute(&mut (*ring).slot);
                    let slot = slots.offset(current as isize);
                    let buf = NETMAP_BUF(ring, (*slot).buf_idx as isize);
                    let slice = slice::from_raw_parts_mut(buf as *mut u8, packet_size);
                    // packet_size is checked above (buf_size is u16)
                    (*slot).len = packet_size as u16;
                    let result = f(slice); // invoke closure
                    let next = nm_ring_next(ring, current);
                    (*ring).head = next;
                    (*ring).cur = next;
                    if !self.uses_wait || self.send_ready().is_err() {
                        // workaround for https://github.com/luigirizzo/netmap/issues/457
                        self.tx_flush()?;
                    }
                    return result;
                }
            }
            Err(Error::Exhausted)
        }
    }

    pub fn zc_forward(&mut self, from: &mut NetmapDesc) -> Result<()> {
        if self.send_ready().is_err() {
            self.tx_flush()?; // workaround for https://github.com/luigirizzo/netmap/issues/457
        }
        unsafe {
            for i in (*self.nm_desc).first_tx_ring..=(*self.nm_desc).last_tx_ring {
                let dst_ring = NETMAP_TXRING((*self.nm_desc).nifp, i as isize);
                if nm_ring_empty(dst_ring) {
                    continue;
                } else {
                    let dst_slots: *mut netmap_slot = mem::transmute(&mut (*dst_ring).slot);
                    let dst = dst_slots.offset((*dst_ring).cur as isize);
                    if let Some(mut src) = from.zc_rx_slot {
                        let tmp = (*dst).buf_idx;
                        (*dst).buf_idx = (*src).buf_idx;
                        (*dst).len = (*src).len;
                        (*dst).flags = NS_BUF_CHANGED;
                        (*src).buf_idx = tmp;
                        (*src).flags = NS_BUF_CHANGED;
                    } else {
                        return Err(Error::Illegal);
                    }
                    from.zc_rx_slot = None;
                    let next = nm_ring_next(dst_ring, (*dst_ring).cur);
                    (*dst_ring).head = next;
                    (*dst_ring).cur = next;
                    if !self.uses_wait || self.send_ready().is_err() {
                        // workaround for https://github.com/luigirizzo/netmap/issues/457
                        self.tx_flush()?;
                    }
                    return Ok(());
                }
            }
            Err(Error::Exhausted)
        }
    }

    pub fn get_nmreq(&self) -> nmreq {
        unsafe { (*self.nm_desc).req }
    }
}

impl Drop for NetmapDesc {
    fn drop(&mut self) {
        unsafe {
            nm_close(self.nm_desc);
        }
        if self.boxed {
            let _ = unsafe { Box::from_raw(self.nm_desc) };
        }
    }
}

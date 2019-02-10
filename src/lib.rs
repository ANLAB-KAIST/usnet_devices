extern crate libc;
extern crate smoltcp;

#[cfg(feature = "netmap")]
mod netmap;
#[cfg(feature = "netmap")]
mod nm;

mod raw_socket;
mod raw_socket_sys;
mod tap_interface;
mod tap_interface_sys;
mod uds;
mod unixdomainsocket;

#[cfg(feature = "netmap")]
pub use self::netmap::{nmreq, Netmap, RxToken as NetmapRxToken, TxToken as NetmapTxToken};

pub use self::raw_socket::{RawSocket, RxToken as RawSocketRxToken, TxToken as RawSocketTxToken};
pub use self::tap_interface::{
    RxToken as TapInterfaceRxToken, TapInterface, TxToken as TapInterfaceTxToken,
};
pub use self::unixdomainsocket::{
    RxToken as UnixDomainSocketRxToken, TxToken as UnixDomainSocketTxToken, UnixDomainSocket,
};
use std::io;

pub const SMOLTCP_ETHERNET_HEADER: usize = 14;

const SIOCGIFMTU: libc::c_ulong = 0x8921;
const SIOCGIFINDEX: libc::c_ulong = 0x8933;
const ETH_P_ALL: libc::c_short = 0x0003;
const IFF_TAP: libc::c_int = 0x0002;
const IFF_NO_PI: libc::c_int = 0x1000;
const TUNSETIFF: libc::c_ulong = 0x400454CA;

#[repr(C)]
#[derive(Debug)]
struct ifreq {
    ifr_name: [libc::c_char; libc::IF_NAMESIZE],
    ifr_data: libc::c_int, /* ifr_ifindex or ifr_mtu */
}

fn ifreq_for(name: &str) -> ifreq {
    let mut ifreq = ifreq {
        ifr_name: [0; libc::IF_NAMESIZE],
        ifr_data: 0,
    };
    for (i, byte) in name.as_bytes().iter().enumerate() {
        ifreq.ifr_name[i] = *byte as libc::c_char
    }
    ifreq
}

fn ifreq_ioctl(
    lower: libc::c_int,
    ifreq: &mut ifreq,
    cmd: libc::c_ulong,
) -> io::Result<libc::c_int> {
    unsafe {
        let res = libc::ioctl(lower, cmd, ifreq as *mut ifreq);
        if res == -1 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(ifreq.ifr_data)
}

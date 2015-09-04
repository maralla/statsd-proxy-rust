#![allow(dead_code)]

use std::io::{self, Error, ErrorKind};
use std::net::ToSocketAddrs;
use std::os::unix::io::RawFd;

use mio;
use nix;
use nix::sys::socket as sock;
pub use nix::sys::socket::{
    AddressFamily,
    SockType,
    SockAddr,
    InetAddr,
    IpAddr,
    Shutdown
};

fn from_nix_error(err: nix::Error) -> Error {
    Error::from_raw_os_error(err.errno() as i32)
}

fn err_check<T>(err: nix::Error) -> io::Result<Option<T>> {
    use std::io::ErrorKind::WouldBlock;

    let e = from_nix_error(err);

    if let WouldBlock = e.kind() {
        return Ok(None);
    }

    Err(e)
}

fn each_addr<A: ToSocketAddrs, F, T>(addr: A, mut f: F) -> io::Result<T>
    where F: FnMut(&SockAddr) -> io::Result<T>
{
    let mut last_err = None;
    for addr in try!(addr.to_socket_addrs()) {
        match f(&SockAddr::new_inet(InetAddr::from_std(&addr))) {
            Ok(l) => return Ok(l),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        Error::new(ErrorKind::InvalidInput,
                   "could not resolve to any addresses")
    }))
}

#[derive(Clone, Eq, PartialEq)]
pub struct Socket {
    fd: RawFd,
}

impl Socket {
    pub fn new(family: AddressFamily, ty: SockType, nonblock: bool) -> io::Result<Socket> {
        let opts = if nonblock {
            sock::SOCK_NONBLOCK | sock::SOCK_CLOEXEC
        } else {
            sock::SOCK_CLOEXEC
        };

        let fd = try!(sock::socket(family, ty, opts).map_err(from_nix_error));

        Ok(Socket::from_rawfd(fd))
    }

    pub fn bind(&self, addr: &SockAddr) -> io::Result<()> {
        sock::bind(self.fd, addr)
            .map_err(from_nix_error)
    }

    pub fn listen(&self, backlog: usize) -> io::Result<()> {
        sock::listen(self.fd, backlog)
            .map_err(from_nix_error)
    }

    fn from_rawfd(fd: RawFd) -> Socket {
        Socket{fd: fd}
    }

    pub fn connect(&self, addr: &SockAddr) -> io::Result<bool> {
        match sock::connect(self.fd, addr) {
            Ok(_) => Ok(true),
            Err(e) => {
                match e {
                    nix::Error::Sys(nix::errno::EINPROGRESS) => Ok(false),
                    _ => Err(from_nix_error(e))
                }
            }
        }
    }

    pub fn accept(&self, nonblock: bool) -> io::Result<Socket> {
        let opts = if nonblock {
            sock::SOCK_NONBLOCK | sock::SOCK_CLOEXEC
        } else {
            sock::SOCK_CLOEXEC
        };

        let fd = try!(sock::accept4(self.fd, opts).map_err(from_nix_error));

        Ok(Socket::from_rawfd(fd))
    }

    pub fn shutdown(&self, how: sock::Shutdown) -> io::Result<()> {
        sock::shutdown(self.fd, how)
            .map_err(from_nix_error)
    }

    pub fn recvfrom(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SockAddr)>> {
        sock::recvfrom(self.fd, buf)
            .map(|n| Some(n))
            .or_else(err_check)
    }

    pub fn sendto(&self, buf: &[u8], target: &SockAddr) -> io::Result<Option<usize>> {
        sock::sendto(self.fd, buf, target, sock::MSG_DONTWAIT)
            .map(|n| Some(n))
            .or_else(err_check)
    }

    pub fn recv(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        sock::recv(self.fd, buf, sock::MSG_DONTWAIT)
            .map(|n| Some(n))
            .or_else(err_check)
    }

    pub fn send(&self, buf: &[u8]) -> io::Result<Option<usize>> {
        sock::send(self.fd, buf, sock::MSG_DONTWAIT)
            .map(|n| Some(n))
            .or_else(err_check)
    }

    pub fn set_reuse(&self) -> io::Result<()> {
        let val = true;

        try!(sock::setsockopt(self.fd, sock::sockopt::ReuseAddr, &val).map_err(from_nix_error));

        sock::setsockopt(self.fd, sock::sockopt::ReusePort, &val)
            .map_err(from_nix_error)
    }
}

impl mio::Evented for Socket {
    fn register(&self, selector: &mut mio::Selector, token: mio::Token,
                interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        selector.register(self.fd, token, interest, opts)
    }

    fn reregister(&self, selector: &mut mio::Selector, token: mio::Token,
                  interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        selector.reregister(self.fd, token, interest, opts)
    }

    fn deregister(&self, selector: &mut mio::Selector) -> io::Result<()> {
        selector.deregister(self.fd)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct UdpStream {
    sock: Socket,
    target: SockAddr
}

impl UdpStream {
    pub fn new<A: ToSocketAddrs>(addr: A) -> io::Result<UdpStream> {
        let sock = try!(Socket::new(AddressFamily::Inet, SockType::Datagram, true));

        let addr = try!(each_addr(addr, |a| Ok(*a)));
        Ok(UdpStream {sock: sock, target: addr})
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<Option<usize>> {
        self.sock.sendto(buf, &self.target)
    }
}

pub struct TcpStream {
    sock: Socket,
}

impl TcpStream {
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<TcpStream> {
        let sock = try!(Socket::new(AddressFamily::Inet, SockType::Stream, true));

        try!(each_addr(addr, |a| sock.connect(a)));

        Ok(TcpStream {sock: sock})
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        self.sock.recv(buf)
    }

    pub fn send(&self, buf: &[u8]) -> io::Result<Option<usize>> {
        self.sock.send(buf)
    }
}

pub struct UdpListener {
    sock: Socket,
}

impl UdpListener {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> io::Result<UdpListener> {
        let sock = try!(Socket::new(AddressFamily::Inet, SockType::Datagram, true));

        try!(sock.set_reuse());
        try!(each_addr(addr, |a| sock.bind(a)));

        Ok(UdpListener {sock: sock})
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SockAddr)>> {
        self.sock.recvfrom(buf)
    }
}

impl mio::Evented for UdpListener {
    fn register(&self, selector: &mut mio::Selector, token: mio::Token,
                interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        self.sock.register(selector, token, interest, opts)
    }

    fn reregister(&self, selector: &mut mio::Selector, token: mio::Token,
                  interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        self.sock.reregister(selector, token, interest, opts)
    }

    fn deregister(&self, selector: &mut mio::Selector) -> io::Result<()> {
        self.sock.deregister(selector)
    }
}

impl mio::Evented for UdpStream {
    fn register(&self, selector: &mut mio::Selector, token: mio::Token,
                interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        self.sock.register(selector, token, interest, opts)
    }

    fn reregister(&self, selector: &mut mio::Selector, token: mio::Token,
                  interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        self.sock.reregister(selector, token, interest, opts)
    }

    fn deregister(&self, selector: &mut mio::Selector) -> io::Result<()> {
        self.sock.deregister(selector)
    }
}

impl mio::Evented for TcpStream {
    fn register(&self, selector: &mut mio::Selector, token: mio::Token,
                interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        self.sock.register(selector, token, interest, opts)
    }

    fn reregister(&self, selector: &mut mio::Selector, token: mio::Token,
                  interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
        self.sock.reregister(selector, token, interest, opts)
    }

    fn deregister(&self, selector: &mut mio::Selector) -> io::Result<()> {
        self.sock.deregister(selector)
    }
}

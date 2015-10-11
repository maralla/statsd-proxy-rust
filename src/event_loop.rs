use mio;
use time;

use std;
use std::io;
use std::net::ToSocketAddrs;
use socket::{UdpListener, TcpStream};
use hash::{ConsistentHash, ServerNode};
use mio::util::Slab;

pub const SERVER: mio::Token = mio::Token(0);
pub const TIMEOUT: mio::Token = mio::Token(1025);

#[allow(dead_code)]
enum State {
    Reading,
    Writing,
    Closed,
}

const HEALTH_PACKET: &'static [u8] = b"health\r\n";
const HEALTH_UP: &'static [u8] = b"health: up";

pub struct Connection {
    stream: TcpStream,
    failure: i32,
    success: i32,
    next_retry: time::Tm,
    token: mio::Token,
    state: State,
    buf: [u8;1024],
    node: ServerNode,
}

impl Connection {
    pub fn new(node: ServerNode, token: mio::Token) -> Connection {
        Connection {
            stream: TcpStream::new().unwrap(),
            failure: 0,
            success: 0,
            next_retry: time::now(),
            token: token,
            state: State::Writing,
            buf: [0;1024],
            node: node,
        }
    }

    pub fn reset_stream(&mut self) {
        self.stream = TcpStream::new().unwrap();
    }

    pub fn register(&mut self, event_loop: &mut mio::EventLoop<Proxy>) {
        self.stream.connect((&self.node.host as &str, self.node.adminport));
        event_loop.register_opt(&self.stream,
                            self.token, mio::EventSet::all(),
                            mio::PollOpt::oneshot()).unwrap();
    }

    fn reregister(&self, event_loop: &mut mio::EventLoop<Proxy>) {
        let event_set = match self.state {
            State::Reading => mio::EventSet::readable(),
            State::Writing => mio::EventSet::writable(),
            _ => mio::EventSet::none(),
        };

        event_loop.reregister(&self.stream, self.token, event_set, mio::PollOpt::oneshot())
            .unwrap();
    }

    pub fn ready(&mut self, event_loop: &mut mio::EventLoop<Proxy>, events: mio::EventSet) {
        if events.is_error() || events.is_hup() {
            self.on_error();
            self.state = State::Closed;
        }

        println!("events: {:?}", events);
        match self.state {
            State::Closed => {
                println!("closed");
            }
            State::Writing => {
                self.on_write();
                self.state = State::Reading;
                self.reregister(event_loop);
            }
            State::Reading => {
                self.on_read();
                self.state = State::Writing;
            }
        }
    }

    fn on_error(&mut self) {
        self.failure += 1;
        self.stream.shutdown();
    }

    fn on_write(&self) {
        self.stream.write(HEALTH_PACKET);
    }

    fn on_read(&mut self) {
        match self.stream.read(&mut self.buf) {
            Ok(Some(0)) => {
                println!("Connection read 0 bytes");
            }
            Ok(Some(n)) => {
                let data = &self.buf[0..n];
                if !data.starts_with(HEALTH_UP) {
                    self.failure += 1;
                } else {
                    self.success += 1;
                }
            }
            Ok(None) => {
                println!("Not ready");
            }
            Err(e) => {
                println!("ERROR: {}", e);
            }
        }
    }
}

pub struct Proxy {
    server: UdpListener,
    read_buf: Vec<u8>,
    state: State,
    ring: ConsistentHash<ServerNode>,
    health_conns: Slab<Connection>,
    check_interval: u64,
}

impl Proxy {
    pub fn new(server: UdpListener, check_interval: u64, health_conns: Slab<Connection>) -> Proxy {
        let mut ring = ConsistentHash::new();

        for c in health_conns.iter() {
            ring.add(&c.node, 20);
        }

        Proxy {
            server: server,
            read_buf: vec![0;4096],
            state: State::Reading,
            ring: ring,
            health_conns: health_conns,
            check_interval: check_interval
        }
    }

    fn ring_remove(&mut self, node: &ServerNode) {
        self.ring.remove(node);
    }

    fn parse(&mut self, n: usize) {
        let packet = &self.read_buf[0..n];
        match packet.iter().position(|x| *x == b':') {
            None => println!("Wrong format of data."),
            Some(n) => {
                match self.ring.get(&packet[0..n]) {
                    Some(node) => {
                        let _ = node.sock.write(packet).unwrap();
                    }
                    None => println!("No node, skip.")
                }
            }
        };
    }

    fn read(&mut self, event_loop: &mut mio::EventLoop<Proxy>) {
        match self.server.read(&mut self.read_buf) {
            Ok(Some((0, _))) => {
                println!("read 0 bytes");
            }
            Ok(Some((n, _))) => {
                println!("read {} bytes", n);

                self.parse(n);
                self.reregister(event_loop);
            }
            Ok(None) => {
                println!("Proxy None");
                self.reregister(event_loop);
            }
            Err(e) => {
                panic!("err={:?}", e);
            }
        }

    }

    fn reregister(&self, event_loop: &mut mio::EventLoop<Proxy>) {
        let event_set = match self.state {
            State::Reading => mio::EventSet::readable(),
            State::Writing => mio::EventSet::writable(),
            _ => mio::EventSet::none(),
        };

        event_loop.reregister(&self.server, SERVER, event_set, mio::PollOpt::oneshot())
            .unwrap();
    }
}

impl mio::Handler for Proxy {
    type Timeout = mio::Token;
    type Message = ();

    fn ready(&mut self, event_loop: &mut mio::EventLoop<Proxy>,
             token: mio::Token, events: mio::EventSet) {
        match token {
            SERVER => {
                assert!(events.is_readable());
                self.read(event_loop);
            }
            _ => {
                self.health_conns[token].ready(event_loop, events);
            }
        }
    }

    fn timeout(&mut self, event_loop: &mut mio::EventLoop<Proxy>, timeout: mio::Token) {
        for c in self.health_conns.iter_mut() {
            let mut ring = &mut self.ring;
            println!("success: {}, failure: {}", c.success, c.failure);
            let duration = time::now() - c.next_retry;

            if c.failure > 2 {
                ring.remove(&c.node);
                c.failure = 0;
            }

            if duration > time::Duration::seconds(30) {
                c.failure = 0;
            }

            match c.state {
                State::Closed => {
                    c.reset_stream();
                    c.register(event_loop);
                    c.state = State::Writing;
                }
                State::Writing => c.reregister(event_loop),
                _ => ()
            }
        }

        let _ = event_loop.timeout_ms(TIMEOUT, self.check_interval).unwrap();
    }
}

use mio;

use socket::{UdpListener};
use hash::{ConsistentHash, ServerNode};

pub const SERVER: mio::Token = mio::Token(0);

#[allow(dead_code)]
enum State {
    Reading,
    Writing,
    Closed,
}

pub struct Proxy {
    server: UdpListener,
    read_buf: Vec<u8>,
    state: State,
    ring: ConsistentHash<ServerNode>,
}

impl Proxy {
    pub fn new(server: UdpListener, node_conf: Vec<ServerNode>) -> Proxy {
        let mut ring = ConsistentHash::new();
        for node in node_conf.iter() {
            ring.add(node, 20);
        }

        Proxy {
            server: server,
            read_buf: vec![0;4096],
            state: State::Reading,
            ring: ring,
        }
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
                println!("None");
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
    type Timeout = ();
    type Message = ();

    fn ready(&mut self, event_loop: &mut mio::EventLoop<Proxy>,
             token: mio::Token, events: mio::EventSet) {
        match token {
            SERVER => {
                assert!(events.is_readable());
                self.read(event_loop);
            }
            _ => {
                println!("other");
            }
        }
    }

    #[allow(unused_variables)]
    fn tick(&mut self, event_loop: &mut mio::EventLoop<Proxy>) {
    }
}

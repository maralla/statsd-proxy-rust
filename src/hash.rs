pub use conhash::ConsistentHash;
use conhash::Node;

use socket::UdpStream;

#[derive(Clone, Eq, PartialEq)]
pub struct ServerNode {
    pub host: String,
    pub port: u16,
    pub adminport: u16,
    pub sock: UdpStream
}

impl Node for ServerNode {
    fn name(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl ServerNode {
    pub fn new(host: &str, port: u16, adminport: u16) -> ServerNode {
        ServerNode {
            host: host.to_owned(),
            port: port,
            adminport: adminport,
            sock: UdpStream::new((host, port)).unwrap()
        }
    }
}

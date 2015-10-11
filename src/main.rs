extern crate nix;
extern crate mio;
extern crate yaml_rust;
extern crate conhash;
extern crate time;

mod socket;
mod event_loop;
mod hash;

use std::env;
use std::fs::File;
use std::thread;
use std::io::Read;
use yaml_rust::yaml;

use hash::ServerNode;
use socket::{UdpListener, TcpStream};
use event_loop::{Proxy, SERVER, TIMEOUT, Connection};
use mio::util::Slab;

struct Manager {
    host: &'static str,
    port: u16,
    threads: Vec<thread::JoinHandle<()>>,
    nodes: Vec<ServerNode>,
    check_interval: u64
}

impl Manager {
    fn new(host: &'static str, port: u16, check_interval: u64, nodes: Vec<ServerNode>) -> Manager {
        Manager {
            threads: vec![],
            host: host,
            port: port,
            nodes: nodes,
            check_interval: check_interval
        }
    }

    fn run(&mut self) {
        let host = self.host;
        let port = self.port;
        let ci = self.check_interval;

        let nodes = self.nodes.clone();

        let t = thread::spawn(move || {
            let server = UdpListener::bind((host, port)).unwrap();

            let mut event_loop = mio::EventLoop::new().unwrap();
            event_loop.register_opt(
                &server, SERVER,
                mio::EventSet::readable() |
                    mio::EventSet::hup() |
                    mio::EventSet::error(),
                mio::PollOpt::edge()).unwrap();

            let mut health_conns = Slab::new_starting_at(mio::Token(1), 1024);
            for node in nodes.iter() {
                let token = health_conns
                    .insert_with(|t| Connection::new(node.clone(), t))
                    .unwrap();
                health_conns[token].register(&mut event_loop);
            }

            let mut proxy = Proxy::new(server, ci, health_conns);

            println!("running proxy at {}:{}", host, port);
            let _ = event_loop.timeout_ms(TIMEOUT, ci).unwrap();
            event_loop.run(&mut proxy).unwrap();
        });
        self.threads.push(t);
    }

    fn join(self) {
        for t in self.threads.into_iter() {
            let _ = t.join();
        }
    }
}

pub fn main() {
    let args: Vec<_> = env::args().collect();
    let mut s = String::new();
    let mut f = File::open(&args[1]).unwrap();
    f.read_to_string(&mut s).unwrap();

    let docs = yaml::YamlLoader::load_from_str(&s).unwrap();

    assert!(docs.len() >= 1);

    let doc = &docs[0];

    let bind = doc["bind"].as_i64().unwrap_or(8977) as u16;
    let threads = doc["threads"].as_i64().unwrap_or(4);
    let check_interval = doc["check_interval"].as_i64().unwrap_or(1000) as u64;

    let mut nodes: Vec<ServerNode> = Vec::new();
    let node_spec = doc["nodes"].as_hash().unwrap();

    for i in node_spec.values() {
        nodes.push(ServerNode::new(
                i["host"].as_str().unwrap(),
                i["port"].as_i64().unwrap() as u16,
                i["adminport"].as_i64().unwrap() as u16,
            )
        )
    }

    let mut m = Manager::new("0.0.0.0", bind, check_interval, nodes);

    for _ in 0..threads {
        m.run();
    }
    m.join();
}

//! A TCP proxy to the test database that a test cuts or freezes, as a lost or hung link does.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Link {
    Up,
    /// Every connection closes, and new ones close at once, as when the tunnel's ssh dies.
    Cut,
    /// Connections stay open and bytes wait, as when the tunnel's ssh is stopped.
    Frozen,
}

struct Shared {
    link: Mutex<Link>,
    changed: Condvar,
    streams: Mutex<Vec<TcpStream>>,
}

pub struct Proxy {
    /// The database URL through this proxy.
    pub url: String,
    shared: Arc<Shared>,
}

impl Proxy {
    /// A proxy to the host and port of the libpq URL `url`, which names them as `@host:port/`.
    pub fn new(url: &str) -> Proxy {
        let at = url.find('@').expect("the URL names a user") + 1;
        let end = at + url[at..].find('/').expect("the URL names a database");
        let upstream = url[at..end].to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Shared {
            link: Mutex::new(Link::Up),
            changed: Condvar::new(),
            streams: Mutex::new(Vec::new()),
        });
        let accepting = Arc::clone(&shared);
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                if *accepting.link.lock().unwrap() == Link::Cut {
                    continue;
                }
                let Ok(server) = TcpStream::connect(&upstream) else {
                    continue;
                };
                let mut streams = accepting.streams.lock().unwrap();
                streams.extend([client.try_clone().unwrap(), server.try_clone().unwrap()]);
                drop(streams);
                for (from, to) in [
                    (client.try_clone().unwrap(), server.try_clone().unwrap()),
                    (server, client),
                ] {
                    let shared = Arc::clone(&accepting);
                    std::thread::spawn(move || pipe(from, to, &shared));
                }
            }
        });
        Proxy {
            url: format!("{}127.0.0.1:{port}{}", &url[..at], &url[end..]),
            shared,
        }
    }

    pub fn set(&self, link: Link) {
        *self.shared.link.lock().unwrap() = link;
        self.shared.changed.notify_all();
        if link == Link::Cut {
            for stream in self.shared.streams.lock().unwrap().drain(..) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.set(Link::Cut);
    }
}

fn pipe(mut from: TcpStream, mut to: TcpStream, shared: &Shared) {
    let mut buffer = [0; 16 * 1024];
    loop {
        let read = match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let mut link = shared.link.lock().unwrap();
        while *link == Link::Frozen {
            link = shared.changed.wait(link).unwrap();
        }
        if *link == Link::Cut {
            break;
        }
        drop(link);
        if to.write_all(&buffer[..read]).is_err() {
            break;
        }
    }
    let _ = to.shutdown(Shutdown::Both);
}

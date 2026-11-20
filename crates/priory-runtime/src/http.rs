//! A small HTTP/1.1 server for read-only requests: bearer token, one request per connection.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The largest request head read; a longer one gets 431.
const HEAD: usize = 8 * 1024;
const HEADERS: usize = 32;
/// How long a client may take to send its whole head, and to read the reply.
const TIMEOUT: Duration = Duration::from_secs(5);
/// The connections served at once; others wait in the listen backlog.
const WORKERS: usize = 8;

pub struct Reply {
    pub status: u16,
    pub kind: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, body: String) -> Self {
        Reply {
            status,
            kind: "application/json",
            body: body.into_bytes(),
        }
    }

    pub fn error(status: u16, message: &str) -> Self {
        Reply::json(status, serde_json::json!({ "error": message }).to_string())
    }
}

/// Serves `listener` on a fixed pool of threads. Each thread calls `make` once for its own
/// handler, which gets the method and path of each request that carries `token`.
pub fn serve<M, H>(listener: TcpListener, token: String, make: M) -> io::Result<Vec<JoinHandle<()>>>
where
    M: Fn() -> H + Send + Sync + 'static,
    H: FnMut(&str, &str) -> Reply,
{
    let shared = Arc::new((listener, token, make));
    (0..WORKERS)
        .map(|_| {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("priory-http".to_owned())
                .spawn(move || {
                    let (listener, token, make) = &*shared;
                    let mut handle = make();
                    for stream in listener.incoming().flatten() {
                        let _ = answer(stream, token, &mut handle);
                    }
                })
        })
        .collect()
}

fn answer(
    mut stream: TcpStream,
    token: &str,
    handle: &mut impl FnMut(&str, &str) -> Reply,
) -> io::Result<()> {
    stream.set_write_timeout(Some(TIMEOUT))?;
    let reply = match read_head(&mut stream) {
        Ok(head) => respond(&head, token, handle),
        Err(reply) => reply,
    };
    let reason = match reply.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    };
    let extra = match reply.status {
        401 => "WWW-Authenticate: Bearer\r\n",
        405 => "Allow: GET\r\n",
        _ => "",
    };
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         {extra}Connection: close\r\n\r\n",
        reply.status,
        reply.kind,
        reply.body.len()
    )?;
    stream.write_all(&reply.body)?;
    stream.shutdown(Shutdown::Write)?;
    // Closing with unread input resets the connection, which can drop the reply before the client reads it.
    stream.set_nonblocking(true)?;
    let _ = io::copy(&mut (&stream).take(64 * 1024), &mut io::sink());
    Ok(())
}

/// The request head, up to its blank line, or the reply that refuses it.
fn read_head(stream: &mut TcpStream) -> Result<Vec<u8>, Reply> {
    let mut head = vec![0; HEAD];
    let mut len = 0;
    let deadline = Instant::now() + TIMEOUT;
    let late = || Reply::error(408, "the request head took too long");
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return Err(late());
        }
        let read = match stream.read(&mut head[len..]) {
            Ok(0) => return Err(Reply::error(400, "the request ended before its head")),
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(late());
            }
            Err(_) => return Err(Reply::error(400, "the request could not be read")),
        };
        len += read;
        let mut headers = [httparse::EMPTY_HEADER; HEADERS];
        match httparse::Request::new(&mut headers).parse(&head[..len]) {
            Ok(httparse::Status::Complete(end)) => {
                head.truncate(end);
                return Ok(head);
            }
            Ok(httparse::Status::Partial) if len == HEAD => {}
            Ok(httparse::Status::Partial) => continue,
            Err(httparse::Error::TooManyHeaders) => {}
            Err(_) => return Err(Reply::error(400, "the request head does not parse")),
        }
        return Err(Reply::error(431, "the request head is too large"));
    }
}

fn respond(head: &[u8], token: &str, handle: &mut impl FnMut(&str, &str) -> Reply) -> Reply {
    let mut headers = [httparse::EMPTY_HEADER; HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    if request.parse(head).is_err() {
        return Reply::error(400, "the request head does not parse");
    }
    let header = |name: &str| {
        request
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value)
    };
    let bearer = header("authorization")
        .and_then(|value| value.get(..7).zip(value.get(7..)))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case(b"bearer "))
        .map(|(_, sent)| sent);
    if !bearer.is_some_and(|sent| same(sent, token.as_bytes())) {
        return Reply::error(401, "send the token as `Authorization: Bearer <token>`");
    }
    let body = request.headers.iter().any(|header| {
        header.name.eq_ignore_ascii_case("transfer-encoding")
            || (header.name.eq_ignore_ascii_case("content-length") && header.value != b"0")
    });
    if body {
        return Reply::error(413, "requests carry no body");
    }
    let (Some(method), Some(path)) = (request.method, request.path) else {
        return Reply::error(400, "the request line is incomplete");
    };
    handle(method, path)
}

/// Whether `sent` equals `token`, in a time that depends only on `token`'s length.
fn same(sent: &[u8], token: &[u8]) -> bool {
    let mut differ = sent.len() ^ token.len();
    for (i, byte) in token.iter().enumerate() {
        differ |= usize::from(sent.get(i).copied().unwrap_or(0) ^ byte);
    }
    differ == 0
}

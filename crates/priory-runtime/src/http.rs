//! A small HTTP/1.1 server: bearer token for reads, a bounded body for hook paths that authenticate
//! themselves, one request per connection.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The largest request head read; a longer one gets 431.
const HEAD: usize = 8 * 1024;
const HEADERS: usize = 32;
/// How long a client may take to send its whole request, and to read the reply.
const TIMEOUT: Duration = Duration::from_secs(5);
/// The largest body a hook path reads; a longer one gets 413 before any of it is read.
const BODY: usize = 1024 * 1024;
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

pub struct Request<'a> {
    pub method: &'a str,
    /// The path, query included.
    pub path: &'a str,
    headers: &'a [httparse::Header<'a>],
    /// Empty except on a hook path.
    pub body: &'a [u8],
}

impl Request<'_> {
    /// The value of the first header called `name`, in any case.
    pub fn header(&self, name: &str) -> Option<&[u8]> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value)
    }
}

/// Serves `listener` on a fixed pool of threads. Each thread calls `make` once for its own
/// handler, which gets each request that carries `token`, and each `POST` to one of `hooks` with
/// its body and no token: the handler authenticates those itself.
pub fn serve<M, H>(
    listener: TcpListener,
    token: String,
    hooks: &'static [&'static str],
    make: M,
) -> io::Result<Vec<JoinHandle<()>>>
where
    M: Fn() -> H + Send + Sync + 'static,
    H: FnMut(&Request) -> Reply,
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
                        let _ = answer(stream, token, hooks, &mut handle);
                    }
                })
        })
        .collect()
}

fn answer(
    mut stream: TcpStream,
    token: &str,
    hooks: &[&str],
    handle: &mut impl FnMut(&Request) -> Reply,
) -> io::Result<()> {
    stream.set_write_timeout(Some(TIMEOUT))?;
    let deadline = Instant::now() + TIMEOUT;
    let reply = match read_head(&mut stream, deadline) {
        Ok((read, end)) => respond(&mut stream, deadline, &read, end, token, hooks, handle),
        Err(reply) => reply,
    };
    let reason = match reply.status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
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

/// What was read up to the end of the request head, and where the head ends, or the reply that
/// refuses it. Bytes past the head are the start of the body.
fn read_head(stream: &mut TcpStream, deadline: Instant) -> Result<(Vec<u8>, usize), Reply> {
    let mut head = vec![0; HEAD];
    let mut len = 0;
    loop {
        len += read_some(stream, &mut head[len..], deadline, "head")?;
        let mut headers = [httparse::EMPTY_HEADER; HEADERS];
        match httparse::Request::new(&mut headers).parse(&head[..len]) {
            Ok(httparse::Status::Complete(end)) => {
                head.truncate(len);
                return Ok((head, end));
            }
            Ok(httparse::Status::Partial) if len == HEAD => {}
            Ok(httparse::Status::Partial) => continue,
            Err(httparse::Error::TooManyHeaders) => {}
            Err(_) => return Err(Reply::error(400, "the request head does not parse")),
        }
        return Err(Reply::error(431, "the request head is too large"));
    }
}

/// Reads at least one byte into `into` before `deadline`, or the reply that refuses the request.
fn read_some(
    stream: &mut TcpStream,
    into: &mut [u8],
    deadline: Instant,
    part: &str,
) -> Result<usize, Reply> {
    let late = || Reply::error(408, &format!("the request {part} took too long"));
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
        return Err(late());
    }
    match stream.read(into) {
        Ok(0) => Err(Reply::error(
            400,
            &format!("the request ended before its {part}"),
        )),
        Ok(read) => Ok(read),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Err(late())
        }
        Err(_) => Err(Reply::error(400, "the request could not be read")),
    }
}

/// The body `Content-Length` announces, `start` being its first bytes, read before `deadline`.
/// A body over `BODY` is refused before any more of it is read.
fn read_body(
    stream: &mut TcpStream,
    deadline: Instant,
    request: &Request,
    start: &[u8],
) -> Result<Vec<u8>, Reply> {
    let length = "send the body with a Content-Length";
    if request.header("transfer-encoding").is_some() {
        return Err(Reply::error(411, length));
    }
    let announced = request
        .header("content-length")
        .and_then(|value| std::str::from_utf8(value).ok()?.parse::<usize>().ok())
        .ok_or_else(|| Reply::error(411, length))?;
    if announced > BODY {
        return Err(Reply::error(413, "the body is over 1 MiB"));
    }
    if start.len() > announced {
        return Err(Reply::error(
            400,
            "the request is longer than its Content-Length",
        ));
    }
    let mut body = start.to_vec();
    body.resize(announced, 0);
    let mut len = start.len();
    while len < announced {
        len += read_some(stream, &mut body[len..], deadline, "body")?;
    }
    Ok(body)
}

fn respond(
    stream: &mut TcpStream,
    deadline: Instant,
    read: &[u8],
    end: usize,
    token: &str,
    hooks: &[&str],
    handle: &mut impl FnMut(&Request) -> Reply,
) -> Reply {
    let mut headers = [httparse::EMPTY_HEADER; HEADERS];
    let mut parsed = httparse::Request::new(&mut headers);
    if parsed.parse(&read[..end]).is_err() {
        return Reply::error(400, "the request head does not parse");
    }
    let (Some(method), Some(path)) = (parsed.method, parsed.path) else {
        return Reply::error(400, "the request line is incomplete");
    };
    let request = Request {
        method,
        path,
        headers: parsed.headers,
        body: &[],
    };
    let bare = path.split_once('?').map_or(path, |(path, _)| path);
    if method == "POST" && hooks.contains(&bare) {
        return match read_body(stream, deadline, &request, &read[end..]) {
            Ok(body) => handle(&Request {
                body: &body,
                ..request
            }),
            Err(reply) => reply,
        };
    }
    let header = |name: &str| request.header(name);
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
    handle(&request)
}

/// Whether `sent` equals `token`, in a time that depends only on `token`'s length.
fn same(sent: &[u8], token: &[u8]) -> bool {
    let mut differ = sent.len() ^ token.len();
    for (i, byte) in token.iter().enumerate() {
        differ |= usize::from(sent.get(i).copied().unwrap_or(0) ^ byte);
    }
    differ == 0
}

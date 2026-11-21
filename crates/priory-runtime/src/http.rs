//! A small HTTP/1.1 server: bearer token for reads and writes, a bounded body for writes and for
//! hook paths that authenticate themselves, one request per connection.

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
/// How long a client may take to send the first byte of its head, so silent ones free a worker soon.
const FIRST_BYTE: Duration = Duration::from_secs(1);
/// The largest body a hook path reads; a longer one gets 413 before any of it is read.
const BODY: usize = 1024 * 1024;
/// The largest body a `POST` with the token reads.
const WRITE: usize = 16 * 1024;
/// The connections served at once; others wait in the listen backlog.
pub const WORKERS: usize = 8;

pub struct Reply {
    pub status: u16,
    pub kind: &'static str,
    /// Further header lines, each ending in `\r\n`.
    pub headers: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, body: String) -> Self {
        Reply {
            status,
            kind: "application/json",
            headers: "",
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
    /// Empty except on a `POST`.
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
/// its body and no token: the handler authenticates those itself. A request to a `public` path
/// needs no token either. A `POST` from a page of another origin gets 403.
pub fn serve<M, H>(
    listener: TcpListener,
    token: String,
    hooks: &'static [&'static str],
    public: fn(&str) -> bool,
    make: M,
) -> io::Result<Vec<JoinHandle<()>>>
where
    M: Fn() -> H + Send + Sync + 'static,
    H: FnMut(&Request) -> Reply,
{
    let gate = Gate {
        token,
        hooks,
        public,
    };
    let shared = Arc::new((listener, gate, make));
    (0..WORKERS)
        .map(|_| {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("priory-http".to_owned())
                .spawn(move || {
                    let (listener, gate, make) = &*shared;
                    let mut handle = make();
                    for stream in listener.incoming().flatten() {
                        let _ = answer(stream, gate, &mut handle);
                    }
                })
        })
        .collect()
}

/// What lets a request through: the token, or a hook or public path.
struct Gate {
    token: String,
    hooks: &'static [&'static str],
    public: fn(&str) -> bool,
}

fn answer(
    mut stream: TcpStream,
    gate: &Gate,
    handle: &mut impl FnMut(&Request) -> Reply,
) -> io::Result<()> {
    stream.set_write_timeout(Some(TIMEOUT))?;
    let deadline = Instant::now() + TIMEOUT;
    let reply = match read_head(&mut stream, deadline) {
        // A handler bug answers 500 instead of ending the worker, so none can drain the pool.
        Ok((read, end)) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            respond(&mut stream, deadline, &read, end, gate, handle)
        }))
        .unwrap_or_else(|_| Reply::error(500, "the request failed inside Priory")),
        Err(reply) => reply,
    };
    let reason = match reply.status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    };
    let extra = match reply.status {
        401 => "WWW-Authenticate: Bearer\r\n",
        405 if reply.headers.is_empty() => "Allow: GET\r\n",
        503 => "Retry-After: 1\r\n",
        _ => "",
    };
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         {extra}{}Connection: close\r\n\r\n",
        reply.status,
        reply.kind,
        reply.body.len(),
        reply.headers
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
    let first = deadline.min(Instant::now() + FIRST_BYTE);
    loop {
        let by = if len == 0 { first } else { deadline };
        len += read_some(stream, &mut head[len..], by, "head")?;
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
/// A body over `limit` is refused before any more of it is read.
fn read_body(
    stream: &mut TcpStream,
    deadline: Instant,
    request: &Request,
    start: &[u8],
    limit: usize,
) -> Result<Vec<u8>, Reply> {
    let length = "send the body with a Content-Length";
    if request.header("transfer-encoding").is_some() {
        return Err(Reply::error(411, length));
    }
    let lengths = request
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("content-length"))
        .count();
    if lengths > 1 {
        return Err(Reply::error(400, "send one Content-Length"));
    }
    let announced = request
        .header("content-length")
        .and_then(|value| std::str::from_utf8(value).ok()?.parse::<usize>().ok())
        .ok_or_else(|| Reply::error(411, length))?;
    if announced > limit {
        let size = match limit {
            BODY => "1 MiB".to_owned(),
            _ => format!("{} KiB", limit / 1024),
        };
        return Err(Reply::error(413, &format!("the body is over {size}")));
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
    gate: &Gate,
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
    if !path.starts_with('/') {
        return Reply::error(400, "the request target is not a path that starts with /");
    }
    let request = Request {
        method,
        path,
        headers: parsed.headers,
        body: &[],
    };
    let bare = path.split_once('?').map_or(path, |(path, _)| path);
    if method == "POST" && !same_origin(&request) {
        return Reply::error(403, "a page of another origin cannot post here");
    }
    if method == "POST" && gate.hooks.contains(&bare) {
        return match read_body(stream, deadline, &request, &read[end..], BODY) {
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
    if !(gate.public)(bare) && !bearer.is_some_and(|sent| same(sent, gate.token.as_bytes())) {
        return Reply::error(401, "send the token as `Authorization: Bearer <token>`");
    }
    if method == "POST" {
        return match read_body(stream, deadline, &request, &read[end..], WRITE) {
            Ok(body) => handle(&Request {
                body: &body,
                ..request
            }),
            Err(reply) => reply,
        };
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

/// Whether the request has no `Origin`, as from curl or a webhook, or one whose host and port are
/// the `Host` it was sent to. The scheme is not compared, so a TLS proxy that keeps `Host` passes.
fn same_origin(request: &Request) -> bool {
    let Some(origin) = request.header("origin") else {
        return true;
    };
    let authority = origin
        .strip_prefix(b"http://")
        .or_else(|| origin.strip_prefix(b"https://"));
    authority
        .zip(request.header("host"))
        .is_some_and(|(authority, host)| authority.eq_ignore_ascii_case(host))
}

/// Whether `sent` equals `token`, in a time that depends only on `token`'s length.
fn same(sent: &[u8], token: &[u8]) -> bool {
    let mut differ = sent.len() ^ token.len();
    for (i, byte) in token.iter().enumerate() {
        differ |= usize::from(sent.get(i).copied().unwrap_or(0) ^ byte);
    }
    differ == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(address: std::net::SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nAuthorization: Bearer t\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response.get(9..12).unwrap_or("none").to_owned()
    }

    #[test]
    fn a_handler_that_panics_answers_500_and_its_worker_keeps_serving() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handler = || {
            |request: &Request| {
                assert_ne!(request.path, "/panic", "a handler bug");
                Reply::json(200, "{}".to_owned())
            }
        };
        serve(listener, "t".to_owned(), &[], |_| false, handler).unwrap();

        for _ in 0..=WORKERS {
            assert_eq!(status(address, "/panic"), "500");
        }
        assert_eq!(status(address, "/fine"), "200");
    }

    use proptest::prelude::*;

    const VALID: &[u8] = b"POST /hook?x=1 HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer t\r\nContent-Length: 4\r\n\r\nbody";

    /// What a client sends: raw bytes, or a valid request cut, spliced or padded.
    fn wire() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            proptest::collection::vec(any::<u8>(), 0..2048),
            (
                0..VALID.len(),
                proptest::collection::vec(any::<u8>(), 0..64)
            )
                .prop_map(|(cut, junk)| [&VALID[..cut], &junk[..], &VALID[cut..]].concat()),
            (0..VALID.len()).prop_map(|cut| VALID[..cut].to_vec()),
            (1..20_000usize)
                .prop_map(|n| [&b"GET /"[..], &vec![b'a'; n], b" HTTP/1.1\r\n\r\n"].concat()),
            (1..60usize).prop_map(|n| [
                &b"GET / HTTP/1.1\r\n"[..],
                &b"A: b\r\n".repeat(n),
                b"\r\n"
            ]
            .concat()),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn any_bytes_get_an_http_status_and_leave_every_worker_serving(bytes in wire()) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            serve(listener, "t".to_owned(), &["/hook"], |_| false, || {
                |_: &Request| Reply::json(200, "{}".to_owned())
            })
            .unwrap();

            let mut stream = TcpStream::connect(address).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let _ = stream.write_all(&bytes);
            let _ = stream.shutdown(Shutdown::Write);
            let mut response = Vec::new();
            let _ = stream.read_to_end(&mut response);
            prop_assert!(response.starts_with(b"HTTP/1.1 "), "no status line");
            prop_assert_eq!(status(address, "/fine"), "200");
        }
    }
}

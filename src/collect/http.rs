//! Answering HTTP over std's `TcpListener`: taking connections a bounded few
//! at a time, reading a request's head within a limit, answering, and
//! closing a connection so the answer reaches the sender.
//!
//! Every limit on what a request may cost is one set here or by the caller,
//! which is why there is no server crate between this and the socket.

use std::borrow::Cow;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use flate2::write::GzEncoder;
use flate2::Compression;

/// The most of a request's line and headers read before they must have
/// ended.
pub(crate) const LONGEST_HEAD: usize = 16 * 1024;

/// The most headers one request may carry.
const MOST_HEADERS: usize = 64;

/// How long a request turned away as busy is read and dropped before it is
/// closed. Whoever turned it away spends it, so it is short.
pub(crate) const LINGER: Duration = Duration::from_millis(100);

/// Take connections on `listener` for as long as the process runs, each on a
/// thread of its own handed to `answer`. A connection arriving while `most`
/// are being answered is handed to `busy` instead, on the accepting thread,
/// and closed.
pub(crate) fn accept(
    listener: TcpListener,
    most: usize,
    busy: impl Fn(&TcpStream) + Send + 'static,
    answer: impl Fn(TcpStream) + Send + Sync + 'static,
) {
    let answer = Arc::new(answer);
    let answering = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if answering.fetch_add(1, Ordering::SeqCst) >= most {
                answering.fetch_sub(1, Ordering::SeqCst);
                busy(&stream);
                close(stream, Instant::now() + LINGER);
                continue;
            }
            let answered = Answering(Arc::clone(&answering));
            let answer = Arc::clone(&answer);
            thread::spawn(move || {
                answer(stream);
                drop(answered);
            });
        }
    });
}

/// One request being answered, counted until it is dropped, panic or not.
struct Answering(Arc<AtomicUsize>);

impl Drop for Answering {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Answer with `status` and `body`, as `kind`, taking no longer than
/// `patience` to send it.
pub(crate) fn reply(stream: &TcpStream, status: &str, kind: &str, body: &[u8], patience: Duration) {
    answer_with(stream, status, kind, "", body, patience);
}

/// Answer as [`reply`] does, with `body` sent in `encoding`, the one the
/// request was found to take.
pub(crate) fn reply_in(
    stream: &TcpStream,
    status: &str,
    kind: &str,
    body: &[u8],
    encoding: Encoding,
    patience: Duration,
) {
    let body = match encoding {
        Encoding::Identity => Cow::Borrowed(body),
        Encoding::Gzip => {
            let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
            Cow::Owned(
                gzip.write_all(body)
                    .and_then(|()| gzip.finish())
                    .expect("deflating into memory cannot fail"),
            )
        }
    };
    answer_with(stream, status, kind, encoding.headers(), &body, patience);
}

fn answer_with(
    stream: &TcpStream,
    status: &str,
    kind: &str,
    headers: &str,
    body: &[u8],
    patience: Duration,
) {
    let mut hurried = Hurried {
        stream,
        deadline: Instant::now() + patience,
    };
    let _ = hurried
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
                 {headers}Connection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .and_then(|()| hurried.write_all(body));
}

/// How an answer's body goes on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding {
    Identity,
    Gzip,
}

impl Encoding {
    /// The header lines saying how the body is encoded, and that a request
    /// taking another encoding may be answered otherwise.
    pub(crate) fn headers(self) -> &'static str {
        match self {
            Encoding::Identity => "Vary: Accept-Encoding\r\n",
            Encoding::Gzip => "Content-Encoding: gzip\r\nVary: Accept-Encoding\r\n",
        }
    }
}

/// A body sent a part at a time for as long as the connection lasts, each
/// part encoded so the reader can decode it the moment it arrives.
pub(crate) struct Parts(Option<GzEncoder<Vec<u8>>>);

impl Parts {
    pub(crate) fn new(encoding: Encoding) -> Self {
        Self(match encoding {
            Encoding::Identity => None,
            Encoding::Gzip => Some(GzEncoder::new(Vec::new(), Compression::default())),
        })
    }

    /// `part` as it goes on the wire. A gzipped part is flushed whole, so
    /// nothing of it waits for the parts after it.
    pub(crate) fn encode<'a>(&mut self, part: &'a [u8]) -> Cow<'a, [u8]> {
        let Some(gzip) = &mut self.0 else {
            return Cow::Borrowed(part);
        };
        gzip.write_all(part)
            .and_then(|()| gzip.flush())
            .expect("deflating into memory cannot fail");
        Cow::Owned(std::mem::take(gzip.get_mut()))
    }
}

/// Send `bytes` down `stream`, taking no longer than `patience`, and say
/// whether they went.
pub(crate) fn send(stream: &TcpStream, bytes: &[u8], patience: Duration) -> std::io::Result<()> {
    Hurried {
        stream,
        deadline: Instant::now() + patience,
    }
    .write_all(bytes)
}

/// A connection written to only until its deadline, however slowly it reads.
struct Hurried<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Write for Hurried<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.stream
            .set_write_timeout(Some(left_until(self.deadline)?))?;
        self.stream.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What is left until `deadline`, or a timeout where nothing is.
fn left_until(deadline: Instant) -> std::io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(ErrorKind::TimedOut.into());
    }
    Ok(left)
}

/// Close `stream` once its answer is sent, first reading and dropping
/// whatever the sender is still sending until it stops or `until`. Closing
/// with bytes unread makes the system reset the connection, and a reset can
/// reach the sender before the answer does.
pub(crate) fn close(stream: TcpStream, until: Instant) {
    let _ = stream.shutdown(Shutdown::Write);
    let _ = std::io::copy(
        &mut Patient {
            stream,
            deadline: until,
        },
        &mut std::io::sink(),
    );
}

/// A connection read only until its deadline, however slowly it sends.
pub(crate) struct Patient {
    pub(crate) stream: TcpStream,
    pub(crate) deadline: Instant,
}

impl Read for Patient {
    fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
        self.stream
            .set_read_timeout(Some(left_until(self.deadline)?))?;
        self.stream.read(into)
    }
}

/// What a request's line and headers say.
pub(crate) struct Head {
    /// Where in what was read the head ends and the body begins.
    pub(crate) end: usize,
    pub(crate) method: String,
    /// The path with its query, as the request line gives it.
    pub(crate) target: String,
    /// Each header's name, and its value where that is text.
    headers: Vec<(String, Option<String>)>,
}

impl Head {
    /// The path without its query.
    pub(crate) fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    /// The value of the first header of this name, in any case, where that
    /// value is text.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(named, _)| named.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value.as_deref())
    }

    /// gzip where the request's Accept-Encoding fields take it at a weight
    /// above nothing, by name or else by `*`, and the body as it is otherwise.
    pub(crate) fn encoding(&self) -> Encoding {
        let codings: Vec<(&str, bool)> = self
            .headers
            .iter()
            .filter(|(named, _)| named.eq_ignore_ascii_case("Accept-Encoding"))
            .filter_map(|(_, value)| value.as_deref())
            .flat_map(|accepted| accepted.split(','))
            .map(|coding| {
                let mut said = coding.split(';').map(str::trim);
                let name = said.next().unwrap_or_default();
                let taken = said.all(|parameter| match parameter.split_once('=') {
                    Some((key, weight)) if key.trim().eq_ignore_ascii_case("q") => weight
                        .trim()
                        .parse::<f32>()
                        .is_ok_and(|weight| weight > 0.0),
                    _ => true,
                });
                (name, taken)
            })
            .collect();
        let taken_as = |names: &[&str]| {
            codings
                .iter()
                .find(|(name, _)| names.iter().any(|named| name.eq_ignore_ascii_case(named)))
                .map(|(_, taken)| *taken)
        };
        if taken_as(&["gzip", "x-gzip"]).or_else(|| taken_as(&["*"])) == Some(true) {
            Encoding::Gzip
        } else {
            Encoding::Identity
        }
    }

    /// The value of the one header of this name, in any case, where there is
    /// exactly one and its value is text.
    pub(crate) fn only(&self, name: &str) -> Option<&str> {
        let mut named = self
            .headers
            .iter()
            .filter(|(named, _)| named.eq_ignore_ascii_case(name));
        match (named.next(), named.next()) {
            (Some((_, value)), None) => value.as_deref(),
            _ => None,
        }
    }
}

/// Why a request's head was not read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// It ran past [`LONGEST_HEAD`] before it was whole.
    TooLong,
    /// It ended or failed before it was whole, or is not HTTP.
    Unreadable,
}

/// Read from `from` into `held` until the request's head is whole, and say
/// what it holds.
pub(crate) fn head(from: &mut impl Read, held: &mut Vec<u8>) -> Result<Head, Unread> {
    let mut chunk = [0; 4096];
    loop {
        let got = from
            .read(&mut chunk)
            .ok()
            .filter(|&got| got > 0)
            .ok_or(Unread::Unreadable)?;
        held.extend_from_slice(&chunk[..got]);
        let mut headers = [httparse::EMPTY_HEADER; MOST_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        let end = match request.parse(held).map_err(|_| Unread::Unreadable)? {
            httparse::Status::Complete(end) if end <= LONGEST_HEAD => end,
            httparse::Status::Complete(_) => return Err(Unread::TooLong),
            httparse::Status::Partial if held.len() < LONGEST_HEAD => continue,
            httparse::Status::Partial => return Err(Unread::TooLong),
        };
        return Ok(Head {
            end,
            method: request.method.unwrap_or_default().to_string(),
            target: request.path.unwrap_or_default().to_string(),
            headers: request
                .headers
                .iter()
                .map(|header| {
                    let value = std::str::from_utf8(header.value).ok();
                    (header.name.to_string(), value.map(str::to_string))
                })
                .collect(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoding_for(accepting: &str) -> Encoding {
        let sent = format!("GET / HTTP/1.1\r\nHost: localhost\r\n{accepting}\r\n\r\n");
        head(&mut sent.as_bytes(), &mut Vec::new())
            .expect("a head")
            .encoding()
    }

    #[test]
    fn gzip_is_sent_only_where_the_request_takes_it() {
        for taking in [
            "Accept-Encoding: gzip",
            "Accept-Encoding: gzip, deflate, br",
            "Accept-Encoding: br;q=1.0, GZIP;q=0.5",
            "Accept-Encoding: x-gzip",
            "accept-encoding: deflate , gzip ; q = 0.1",
            "Accept-Encoding: br\r\nAccept-Encoding: gzip, identity;q=0",
            "Accept-Encoding: *;q=1, identity;q=0",
            "Accept-Encoding: br, *",
        ] {
            assert_eq!(encoding_for(taking), Encoding::Gzip, "{taking}");
        }
        for refusing in [
            "Accept: text/html",
            "Accept-Encoding: identity",
            "Accept-Encoding: br, deflate",
            "Accept-Encoding: gzip;q=0",
            "Accept-Encoding: gzip;q=0.000",
            "Accept-Encoding: gzip;q=nonsense",
            "Accept-Encoding: gzipped",
            "Accept-Encoding: *;q=0",
            "Accept-Encoding: gzip;q=0, *",
        ] {
            assert_eq!(encoding_for(refusing), Encoding::Identity, "{refusing}");
        }
    }

    /// A reader holding only the parts sent so far decodes every one of
    /// them, so no part waits on the next to be read.
    #[test]
    fn each_gzipped_part_decodes_the_moment_it_arrives() {
        let mut parts = Parts::new(Encoding::Gzip);
        let mut reader = flate2::write::GzDecoder::new(Vec::new());
        let mut sent = Vec::new();
        for part in [
            "event: page\ndata: <p>\n\n",
            ":\n\n",
            "event: patch\ndata: 0 1 1\n\n",
        ] {
            reader
                .write_all(&parts.encode(part.as_bytes()))
                .and_then(|()| reader.flush())
                .expect("a part decodes");
            sent.extend_from_slice(part.as_bytes());
            assert_eq!(reader.get_ref(), &sent);
        }
    }

    #[test]
    fn a_part_sent_as_it_is_goes_unchanged() {
        assert_eq!(&*Parts::new(Encoding::Identity).encode(b":\n\n"), b":\n\n");
    }

    /// A reader taking a little at a time keeps every single write moving,
    /// so only a deadline on the whole answer stops it in time.
    #[test]
    fn an_answer_read_slowly_is_given_up_at_its_patience() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let sender =
            TcpStream::connect(listener.local_addr().expect("an address")).expect("a connection");
        let (answering, _) = listener.accept().expect("the connection");
        let reader = thread::spawn(move || {
            let gives_up = Instant::now() + Duration::from_secs(5);
            let mut sender = sender;
            let mut chunk = vec![0; 1024 * 1024];
            while Instant::now() < gives_up && sender.read(&mut chunk).is_ok_and(|got| got > 0) {
                thread::sleep(Duration::from_millis(50));
            }
        });

        let started = Instant::now();
        reply(
            &answering,
            "200 OK",
            "text/plain",
            &vec![b'x'; 64 * 1024 * 1024],
            Duration::from_millis(300),
        );
        let took = started.elapsed();
        drop(answering);
        reader.join().expect("the reader");

        assert!(took < Duration::from_secs(2), "the answer took {took:?}");
    }
}

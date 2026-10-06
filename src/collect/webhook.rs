//! GitHub's webhook deliveries, taken over HTTP. A delivery is only a
//! trigger: one signed with the shared secret and about a pull request names
//! the pull request to settle, and nothing else in it is read.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::collect::gates::PullRequest;

/// The path a readiness probe asks, answered whatever the secret.
pub const HEALTH: &str = "/healthz";

/// The most of a body read before its signature is checked, so the most an
/// unsigned request can make the listener hold. A pull_request delivery is
/// far smaller: its largest field, the pull request's own body, is held by
/// GitHub to 65,536 characters.
const LARGEST_DELIVERY: usize = 1024 * 1024;

/// The most of a request's line and headers read before they must have
/// ended. GitHub's come to under a kilobyte.
const LONGEST_HEAD: usize = 16 * 1024;

/// The most headers one request may carry.
const MOST_HEADERS: usize = 64;

/// The most requests answered at once, which with [`LARGEST_DELIVERY`]
/// bounds the memory every request in flight can hold between them.
const MOST_AT_ONCE: usize = 8;

/// The most of what was heard that waits to be settled or reported, which
/// bounds what requests can leave behind them while settling is slow. Where
/// it is full, a delivery to settle is told to come back later, and a refusal
/// goes unreported.
pub const MOST_WAITING: usize = 64;

/// How a request is told to come back later.
const BUSY: (&str, &str) = ("503 Service Unavailable", "busy\n");

/// How long a request has to arrive in full, which is how long a sender that
/// goes quiet holds one of the [`MOST_AT_ONCE`]. GitHub itself gives up on an
/// answer after ten seconds.
const PATIENCE: Duration = Duration::from_secs(10);

/// How long a request turned away while the listener is busy is read and
/// dropped before it is closed. The accepting thread spends it, so it is
/// short.
const LINGER: Duration = Duration::from_millis(100);

/// The secret GitHub signs each delivery with.
pub struct Secret(Vec<u8>);

impl Secret {
    /// `text` without the whitespace around it, which a file holding a
    /// secret usually ends with, or nothing where that leaves no secret at
    /// all.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty()).then(|| Self(text.as_bytes().to_vec()))
    }
}

/// What the listener made of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    /// A readiness probe.
    Healthy,
    /// A signed delivery about this pull request.
    Settle(PullRequest),
    /// A signed delivery about something other than a pull request.
    Ignored,
    /// A delivery carrying no signature.
    Unsigned,
    /// A delivery whose signature the secret did not make.
    Forged,
    /// A signed pull_request delivery whose payload names no repository and
    /// number.
    NamesNoPullRequest,
    /// A body larger than any pull_request delivery.
    TooLarge,
    /// A delivery that does not give its length up front, which a chunked
    /// one does not.
    Unmeasured,
    /// A path or method the listener does not answer.
    Unknown,
}

impl Heard {
    /// Whether what was heard goes on to be settled or reported.
    fn handed_on(&self) -> bool {
        !matches!(self, Heard::Healthy | Heard::Unknown)
    }

    /// The status line and text the request is answered with, given whether
    /// what was heard found room to be handed on.
    fn answer(&self, room: bool) -> (&'static str, &'static str) {
        match self {
            Heard::Healthy => ("200 OK", "ok\n"),
            Heard::Settle(_) if !room => BUSY,
            Heard::Settle(_) => ("202 Accepted", "settling\n"),
            Heard::Ignored => ("202 Accepted", "ignored\n"),
            Heard::Unsigned | Heard::Forged => ("401 Unauthorized", "signature refused\n"),
            Heard::NamesNoPullRequest => ("400 Bad Request", "no pull request named\n"),
            Heard::TooLarge => ("413 Content Too Large", "too large\n"),
            Heard::Unmeasured => ("411 Length Required", "length required\n"),
            Heard::Unknown => ("404 Not Found", "not found\n"),
        }
    }
}

/// What one delivery, `body` under the `X-GitHub-Event` and
/// `X-Hub-Signature-256` headers it came with, comes to. The signature is
/// checked before anything else is read, whatever the event.
pub fn delivery(
    secret: &Secret,
    event: Option<&str>,
    signature: Option<&str>,
    body: &[u8],
) -> Heard {
    let Some(signature) = signature else {
        return Heard::Unsigned;
    };
    if !signed(secret, signature, body) {
        return Heard::Forged;
    }
    if event != Some("pull_request") {
        return Heard::Ignored;
    }
    match serde_json::from_slice::<PullRequestEvent>(body) {
        Ok(event) => Heard::Settle(PullRequest {
            repo: event.repository.full_name,
            number: event.number,
        }),
        Err(_) => Heard::NamesNoPullRequest,
    }
}

/// The two fields of a pull_request delivery that name its pull request.
#[derive(Deserialize)]
struct PullRequestEvent {
    number: u64,
    repository: Repository,
}

#[derive(Deserialize)]
struct Repository {
    full_name: String,
}

/// Whether `signature`, as GitHub writes it, is `secret`'s over `body`.
fn signed(secret: &Secret, signature: &str, body: &[u8]) -> bool {
    let Some(digest) = signature.strip_prefix("sha256=").and_then(bytes_of) else {
        return false;
    };
    let mut mac =
        Hmac::<Sha256>::new_from_slice(&secret.0).expect("an HMAC takes a key of any length");
    mac.update(body);
    mac.verify_slice(&digest).is_ok()
}

/// `hex` as the bytes it spells, or nothing where it spells none.
fn bytes_of(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).ok())
        .collect()
}

/// Take requests on `address`, each on a thread of its own, hand `heard` what
/// each was, unless it was a health check or nothing the listener answers,
/// and answer it at once. A request arriving while
/// [`MOST_AT_ONCE`] are being answered is told to come back later. Gives back
/// the address taken, which names the port where `address` left it to the
/// system.
pub fn listen(
    address: &str,
    secret: Secret,
    heard: SyncSender<Heard>,
) -> anyhow::Result<SocketAddr> {
    let listener = TcpListener::bind(address).with_context(|| format!("listening on {address}"))?;
    let taken = listener
        .local_addr()
        .with_context(|| format!("reading the address {address} gave"))?;
    let secret = Arc::new(secret);
    let answering = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if answering.fetch_add(1, Ordering::SeqCst) >= MOST_AT_ONCE {
                answering.fetch_sub(1, Ordering::SeqCst);
                reply(&stream, BUSY);
                close(stream, Instant::now() + LINGER);
                continue;
            }
            let answered = Answering(Arc::clone(&answering));
            let secret = Arc::clone(&secret);
            let heard = heard.clone();
            thread::spawn(move || {
                answer(stream, &secret, &heard);
                drop(answered);
            });
        }
    });
    Ok(taken)
}

/// One request being answered, counted until it is dropped, panic or not.
struct Answering(Arc<AtomicUsize>);

impl Drop for Answering {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Read one request from `stream` and answer it, or close it unanswered
/// where it never arrives in full or is not HTTP.
fn answer(stream: TcpStream, secret: &Secret, heard: &SyncSender<Heard>) {
    let mut stream = Patient {
        stream,
        deadline: Instant::now() + PATIENCE,
    };
    let Some(said) = read(&mut stream, secret) else {
        return;
    };
    let room = said.handed_on() && heard.try_send(said.clone()).is_ok();
    reply(&stream.stream, said.answer(room));
    close(stream.stream, stream.deadline);
}

fn reply(mut stream: &TcpStream, (status, text): (&str, &str)) {
    let _ = stream.set_write_timeout(Some(PATIENCE));
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{text}",
            text.len()
        )
        .as_bytes(),
    );
}

/// Close `stream` once its answer is sent, first reading and dropping
/// whatever the sender is still sending until it stops or `until`. Closing
/// with bytes unread makes the system reset the connection, and a reset can
/// reach the sender before the answer does.
fn close(stream: TcpStream, until: Instant) {
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
struct Patient {
    stream: TcpStream,
    deadline: Instant,
}

impl Read for Patient {
    fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(into)
    }
}

/// What the one request `from` sends comes to, or nothing where it ends,
/// fails or runs past a limit before it is whole, or is not HTTP.
fn read(from: &mut impl Read, secret: &Secret) -> Option<Heard> {
    let mut held = Vec::new();
    let head = head(from, &mut held)?;
    match (head.method.as_str(), head.path.as_str()) {
        ("GET", HEALTH) => return Some(Heard::Healthy),
        ("POST", _) => {}
        _ => return Some(Heard::Unknown),
    }
    let length = match head.length {
        _ if head.chunked => return Some(Heard::Unmeasured),
        None => return Some(Heard::Unmeasured),
        Some(length) if length > LARGEST_DELIVERY => return Some(Heard::TooLarge),
        Some(length) => length,
    };
    let mut body = held.split_off(head.end);
    if body.len() < length {
        let mut rest = vec![0; length - body.len()];
        from.read_exact(&mut rest).ok()?;
        body.extend_from_slice(&rest);
    }
    body.truncate(length);
    Some(delivery(
        secret,
        head.event.as_deref(),
        head.signature.as_deref(),
        &body,
    ))
}

/// What a request's line and headers say, of what the listener reads.
struct Head {
    /// Where in what was read the head ends and the body begins.
    end: usize,
    method: String,
    /// The path without its query.
    path: String,
    /// The body's length, where the request gives one that is a number.
    length: Option<usize>,
    chunked: bool,
    event: Option<String>,
    signature: Option<String>,
}

/// Read from `from` into `held` until the request's head is whole, and say
/// what it holds.
fn head(from: &mut impl Read, held: &mut Vec<u8>) -> Option<Head> {
    let mut chunk = [0; 4096];
    loop {
        let got = from.read(&mut chunk).ok().filter(|&got| got > 0)?;
        held.extend_from_slice(&chunk[..got]);
        let mut headers = [httparse::EMPTY_HEADER; MOST_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        let end = match request.parse(held).ok()? {
            httparse::Status::Complete(end) => end,
            httparse::Status::Partial if held.len() < LONGEST_HEAD => continue,
            httparse::Status::Partial => return None,
        };
        let value = |name: &str| {
            request
                .headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case(name))
                .and_then(|header| std::str::from_utf8(header.value).ok())
                .map(str::to_string)
        };
        let path = request.path.unwrap_or_default();
        return Some(Head {
            end,
            method: request.method.unwrap_or_default().to_string(),
            path: path.split('?').next().unwrap_or_default().to_string(),
            length: value("Content-Length").and_then(|length| length.trim().parse().ok()),
            chunked: value("Transfer-Encoding").is_some(),
            event: value("X-GitHub-Event"),
            signature: value("X-Hub-Signature-256"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's own worked example of a signature, from *Validating webhook
    /// deliveries*.
    const GITHUBS_SECRET: &str = "It's a Secret to Everybody";
    const GITHUBS_PAYLOAD: &[u8] = b"Hello, World!";
    const GITHUBS_SIGNATURE: &str =
        "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";

    /// A pull_request delivery cut down to what bdi reads and a little of
    /// what it does not.
    const CLOSED_42: &str = r#"{"action":"closed","number":42,"pull_request":{"merged":true},"repository":{"full_name":"example/ark"}}"#;

    fn secret() -> Secret {
        Secret::new("swordfish").expect("a secret")
    }

    fn signature(body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"swordfish").expect("any key");
        mac.update(body);
        let digest: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("sha256={digest}")
    }

    #[test]
    fn githubs_worked_example_is_signed_by_its_secret() {
        let secret = Secret::new(GITHUBS_SECRET).expect("a secret");
        assert!(signed(&secret, GITHUBS_SIGNATURE, GITHUBS_PAYLOAD));
        assert!(!signed(&secret, GITHUBS_SIGNATURE, b"Hello, World?"));
    }

    #[test]
    fn a_signed_pull_request_delivery_names_the_pull_request_to_settle() {
        assert_eq!(
            delivery(
                &secret(),
                Some("pull_request"),
                Some(&signature(CLOSED_42.as_bytes())),
                CLOSED_42.as_bytes()
            ),
            Heard::Settle(PullRequest {
                repo: "example/ark".to_string(),
                number: 42,
            })
        );
    }

    #[test]
    fn a_delivery_signed_with_another_secret_is_forged() {
        let other = Secret::new("hunter2").expect("a secret");
        assert_eq!(
            delivery(
                &other,
                Some("pull_request"),
                Some(&signature(CLOSED_42.as_bytes())),
                CLOSED_42.as_bytes()
            ),
            Heard::Forged
        );
    }

    #[test]
    fn a_signature_that_is_not_one_is_forged() {
        for written in [
            "",
            "sha256=",
            "sha1=757107ea0eb2509fc211221cce984b8a37570b6d",
            "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e1",
            "sha256=zz7107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17",
            "sha256=+57107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17",
            "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17ff",
        ] {
            assert_eq!(
                delivery(
                    &secret(),
                    Some("pull_request"),
                    Some(written),
                    CLOSED_42.as_bytes()
                ),
                Heard::Forged,
                "{written:?}"
            );
        }
    }

    #[test]
    fn a_delivery_with_no_signature_is_unsigned_whatever_its_event() {
        for event in [Some("pull_request"), Some("ping"), None] {
            assert_eq!(
                delivery(&secret(), event, None, CLOSED_42.as_bytes()),
                Heard::Unsigned
            );
        }
    }

    #[test]
    fn a_signed_delivery_of_any_other_event_is_ignored() {
        let ping = br#"{"zen":"Keep it logically awesome.","hook_id":1}"#;
        for event in [Some("ping"), Some("push"), None] {
            assert_eq!(
                delivery(&secret(), event, Some(&signature(ping)), ping),
                Heard::Ignored
            );
        }
    }

    #[test]
    fn a_signed_pull_request_delivery_naming_no_pull_request_says_so() {
        let body = br#"{"action":"closed","repository":{"full_name":"example/ark"}}"#;
        assert_eq!(
            delivery(
                &secret(),
                Some("pull_request"),
                Some(&signature(body)),
                body
            ),
            Heard::NamesNoPullRequest
        );
    }

    #[test]
    fn a_secret_is_read_without_the_whitespace_around_it_and_an_empty_one_is_none() {
        let read = Secret::new("  swordfish\n").expect("a secret");
        let body = CLOSED_42.as_bytes();
        assert!(signed(&read, &signature(body), body));
        assert!(Secret::new(" \n").is_none());
    }

    /// What `sent` comes to, read as one request.
    fn heard(sent: &[u8]) -> Option<Heard> {
        read(&mut std::io::Cursor::new(sent), &secret())
    }

    fn posted(headers: &str, body: &str) -> Vec<u8> {
        format!("POST /hook?x=1 HTTP/1.1\r\nHost: bdi\r\n{headers}\r\n{body}").into_bytes()
    }

    #[test]
    fn a_signed_delivery_over_http_is_read_to_the_length_it_gives() {
        let headers = format!(
            "content-length: {}\r\nX-GitHub-Event: pull_request\r\nX-Hub-Signature-256: {}\r\n",
            CLOSED_42.len(),
            signature(CLOSED_42.as_bytes())
        );
        assert_eq!(
            heard(&posted(&headers, &format!("{CLOSED_42}trailing"))),
            Some(Heard::Settle(PullRequest {
                repo: "example/ark".to_string(),
                number: 42,
            }))
        );
    }

    #[test]
    fn a_body_that_ends_before_its_length_is_not_answered() {
        let headers = format!(
            "Content-Length: {}\r\nX-GitHub-Event: pull_request\r\nX-Hub-Signature-256: {}\r\n",
            CLOSED_42.len() + 1,
            signature(CLOSED_42.as_bytes())
        );
        assert_eq!(heard(&posted(&headers, CLOSED_42)), None);
    }

    /// No body follows the head, so a read of it would end the request
    /// unanswered.
    #[test]
    fn a_length_over_a_mebibyte_is_too_large_without_its_body_being_read() {
        assert_eq!(
            heard(&posted("Content-Length: 1048577\r\n", "")),
            Some(Heard::TooLarge)
        );
        assert_eq!(
            heard(&posted("Content-Length: 99999999999999999999999\r\n", "")),
            Some(Heard::Unmeasured),
            "a length that is no number at all"
        );
    }

    #[test]
    fn a_delivery_that_does_not_give_its_length_up_front_is_unmeasured() {
        assert_eq!(
            heard(&posted("Transfer-Encoding: chunked\r\n", "0\r\n\r\n")),
            Some(Heard::Unmeasured)
        );
        assert_eq!(heard(&posted("", "")), Some(Heard::Unmeasured));
    }

    #[test]
    fn only_a_get_of_the_health_path_is_healthy_and_nothing_else_but_a_post_is_answered() {
        assert_eq!(
            heard(b"GET /healthz HTTP/1.1\r\nHost: bdi\r\n\r\n"),
            Some(Heard::Healthy)
        );
        assert_eq!(
            heard(b"GET / HTTP/1.1\r\nHost: bdi\r\n\r\n"),
            Some(Heard::Unknown)
        );
        assert_eq!(
            heard(b"PUT /healthz HTTP/1.1\r\nHost: bdi\r\n\r\n"),
            Some(Heard::Unknown)
        );
    }

    #[test]
    fn what_is_not_http_or_never_finishes_its_head_is_not_answered() {
        assert_eq!(heard(b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03"), None);
        assert_eq!(heard(b"POST /hook HTTP/1.1\r\nHost: bdi\r\n"), None);
        let endless = format!("GET / HTTP/1.1\r\nX-Padding: {}", "a".repeat(LONGEST_HEAD));
        assert_eq!(heard(endless.as_bytes()), None);
    }

    /// A connection to read as the listener does, with `patience` to arrive
    /// in, and the sending end of it.
    fn connected(patience: Duration) -> (Patient, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let sender = TcpStream::connect(listener.local_addr().expect("its address"))
            .expect("the connection");
        let (stream, _) = listener.accept().expect("the other end");
        let reader = Patient {
            stream,
            deadline: Instant::now() + patience,
        };
        (reader, sender)
    }

    /// The body is a signed delivery that would settle #42, but for its last
    /// byte, which the sender never sends.
    #[test]
    fn a_sender_that_goes_quiet_before_its_delivery_is_whole_is_not_answered() {
        let (mut reader, mut sender) = connected(Duration::from_millis(200));
        let sent = posted(
            &format!(
                "Content-Length: {}\r\nX-GitHub-Event: pull_request\r\nX-Hub-Signature-256: {}\r\n",
                CLOSED_42.len(),
                signature(CLOSED_42.as_bytes())
            ),
            CLOSED_42,
        );
        sender
            .write_all(&sent[..sent.len() - 1])
            .expect("all but the last byte");

        let began = Instant::now();
        assert_eq!(read(&mut reader, &secret()), None);
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "{:?}",
            began.elapsed()
        );
    }

    /// Each byte comes well inside the time one read may wait, so only a
    /// deadline on the whole request stops it.
    #[test]
    fn a_sender_dripping_its_delivery_past_the_deadline_is_not_answered() {
        let (mut reader, sender) = connected(Duration::from_millis(300));
        let dripping = thread::spawn(move || {
            let mut sender = sender;
            for byte in b"POST /hook HTTP/1.1\r\nHost: bdi\r\nX-Padding: "
                .iter()
                .cycle()
            {
                if sender.write_all(&[*byte]).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });

        let began = Instant::now();
        assert_eq!(read(&mut reader, &secret()), None);
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "{:?}",
            began.elapsed()
        );
        drop(reader);
        dripping
            .join()
            .expect("the sender stops once the reader has gone");
    }

    #[test]
    fn a_delivery_to_settle_with_no_room_to_wait_is_told_to_come_back_and_a_refusal_is_not() {
        let settle = Heard::Settle(PullRequest {
            repo: "example/ark".to_string(),
            number: 42,
        });
        assert_eq!(settle.answer(true).0, "202 Accepted");
        assert_eq!(settle.answer(false), BUSY);
        assert_eq!(Heard::Forged.answer(false).0, "401 Unauthorized");
        assert!(!Heard::Healthy.handed_on());
        assert!(!Heard::Unknown.handed_on());
        assert!(Heard::Unsigned.handed_on());
    }
}

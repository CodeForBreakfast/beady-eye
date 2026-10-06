//! GitHub's webhook deliveries, taken over HTTP. A delivery is only a
//! trigger: one signed with the shared secret and about a pull request names
//! the pull request to settle, and nothing else in it is read.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;

use anyhow::{anyhow, Context};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tiny_http::{Method, Request, Response, Server};

use crate::collect::gates::PullRequest;

/// The path a readiness probe asks, answered whatever the secret.
pub const HEALTH: &str = "/healthz";

/// GitHub sends no delivery larger than this, so nothing larger is one.
/// Reading stops here, which keeps a body that never ends out of memory.
const LARGEST_DELIVERY: u64 = 25 * 1024 * 1024;

/// The secret GitHub signs each delivery with.
pub struct Secret(Vec<u8>);

impl Secret {
    /// `text` without the whitespace around it, which a file or a variable
    /// holding a secret usually ends with and GitHub's own field drops, or
    /// nothing where that leaves no secret at all.
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
    /// A body larger than any delivery GitHub sends.
    TooLarge,
    /// A path or method the listener does not answer.
    Unknown,
}

impl Heard {
    /// The status and text the request is answered with.
    fn answer(&self) -> (u16, &'static str) {
        match self {
            Heard::Healthy => (200, "ok\n"),
            Heard::Settle(_) => (202, "settling\n"),
            Heard::Ignored => (202, "ignored\n"),
            Heard::Unsigned | Heard::Forged => (401, "signature refused\n"),
            Heard::NamesNoPullRequest => (400, "no pull request named\n"),
            Heard::TooLarge => (413, "too large\n"),
            Heard::Unknown => (404, "not found\n"),
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
    if hex.len() % 2 != 0 || !hex.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).ok())
        .collect()
}

/// Take requests on `address`, each on a thread of its own, answer each at
/// once, and only then hand `heard` what it was, unless it was a health check
/// or nothing the listener answers. Gives back the address taken, which names
/// the port where `address` left it to the system.
pub fn listen(address: &str, secret: Secret, heard: Sender<Heard>) -> anyhow::Result<SocketAddr> {
    let server = Server::http(address).map_err(|e| anyhow!("listening on {address}: {e}"))?;
    let taken = server
        .server_addr()
        .to_ip()
        .with_context(|| format!("{address} is not an IP address and port"))?;
    let secret = Arc::new(secret);
    thread::spawn(move || {
        for request in server.incoming_requests() {
            let secret = Arc::clone(&secret);
            let heard = heard.clone();
            thread::spawn(move || answer(request, &secret, &heard));
        }
    });
    Ok(taken)
}

fn answer(mut request: Request, secret: &Secret, heard: &Sender<Heard>) {
    let path = request.url().split('?').next().unwrap_or_default();
    let said = match (request.method(), path) {
        (Method::Get | Method::Head, HEALTH) => Heard::Healthy,
        (Method::Post, _) => {
            let mut body = Vec::new();
            if request
                .as_reader()
                .take(LARGEST_DELIVERY + 1)
                .read_to_end(&mut body)
                .is_err()
            {
                return;
            }
            if body.len() as u64 > LARGEST_DELIVERY {
                Heard::TooLarge
            } else {
                delivery(
                    secret,
                    header(&request, "X-GitHub-Event"),
                    header(&request, "X-Hub-Signature-256"),
                    &body,
                )
            }
        }
        _ => Heard::Unknown,
    };
    let (status, text) = said.answer();
    let _ = request.respond(Response::from_string(text).with_status_code(status));
    if !matches!(said, Heard::Healthy | Heard::Unknown) {
        let _ = heard.send(said);
    }
}

fn header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
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
}

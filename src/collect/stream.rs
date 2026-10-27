//! A project's `bd serve` event stream, followed so that `bdi watch` hears of
//! each write to the tracker as it happens, whichever bd client made it.
//!
//! What the stream says goes where a producer's lines go. A record says the
//! project changed, and the stream opening or sending its heartbeat vouches
//! that nothing else has. So the debounce, the poll standing down and the
//! lapse of a project nothing vouches for are the ones a producer gets.
//!
//! Each record carries its journal `seq` as the event id, and `since`
//! resumes after it, so a dropped stream picks up where it was. A `since` the
//! journal no longer reaches is refused with a 410 at connect, or ended with
//! a `truncated` event once open, and the project is then read in full once
//! and followed from the journal's head.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use super::changes::{Heard, Report};

/// How long the follower waits on a stream and between attempts at one.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How long a stream may say nothing before it is taken to have dropped.
    pub silent_after: Duration,
    /// How long to wait before connecting again.
    pub again_after: Duration,
}

/// bd serve sends a heartbeat after 20 seconds of silence, and looks for
/// silence about once a second, so a stream quiet for 30 has missed one. Its
/// stream asks a client to wait 3 seconds before connecting again.
pub const BD_SERVE: Timing = Timing {
    silent_after: Duration::from_secs(30),
    again_after: Duration::from_secs(3),
};

/// Follow the stream of the bd serve at `at` for `project` until nothing is
/// left to take what it says.
pub fn follow(project: &str, at: SocketAddr, report: &impl Report, timing: Timing) {
    let changed = || report.report(Heard::Changed(project.to_string()));
    let covered = || report.report(Heard::Covered(project.to_string()));
    let mut resuming = None;
    loop {
        let since = match resuming {
            Some(seq) => seq,
            None => {
                let Some(head) = head(at, timing) else {
                    thread::sleep(timing.again_after);
                    continue;
                };
                // Nothing says what moved before the head, so the project is
                // read in full.
                if !changed() {
                    return;
                }
                head
            }
        };
        resuming = match read_stream(at, since, timing, changed, covered) {
            Ended::NobodyListens => return,
            Ended::Dropped { resume_after } => Some(resume_after),
            Ended::Unresumable => None,
        };
        thread::sleep(timing.again_after);
    }
}

/// How one connection to the stream ended.
enum Ended {
    /// Nothing is left to take what the stream says.
    NobodyListens,
    /// The stream went, and can be resumed after the record last read.
    Dropped { resume_after: u64 },
    /// The journal no longer reaches the record last read.
    Unresumable,
}

/// Say what the stream says from after `since`, until it ends.
fn read_stream(
    at: SocketAddr,
    since: u64,
    timing: Timing,
    changed: impl Fn() -> bool,
    covered: impl Fn() -> bool,
) -> Ended {
    let mut resume_after = since;
    let dropped = |resume_after| Ended::Dropped { resume_after };
    let Some(response) = get(at, &format!("/v0/beads/events:watch?since={since}"), timing) else {
        return dropped(resume_after);
    };
    match response.status {
        200 => {}
        410 => return Ended::Unresumable,
        _ => return dropped(resume_after),
    }
    if !covered() {
        return Ended::NobodyListens;
    }

    let mut id = None;
    let mut truncated = false;
    for line in response.body.lines() {
        let Ok(line) = line else {
            return dropped(resume_after);
        };
        let said = if line.is_empty() {
            if truncated {
                return Ended::Unresumable;
            }
            match id.take() {
                Some(seq) => {
                    resume_after = seq;
                    changed()
                }
                None => true,
            }
        } else if line.starts_with(':') {
            covered()
        } else {
            if let Some(seq) = field(&line, "id") {
                id = seq.parse().ok();
            }
            truncated |= field(&line, "event") == Some("truncated");
            true
        };
        if !said {
            return Ended::NobodyListens;
        }
    }
    dropped(resume_after)
}

/// The value of an event stream line naming `name`, where it does.
fn field<'l>(line: &'l str, name: &str) -> Option<&'l str> {
    let value = line.strip_prefix(name)?.strip_prefix(':')?;
    Some(value.strip_prefix(' ').unwrap_or(value))
}

/// The journal's head: the `seq` of the last record it holds. A journal
/// pruned past the start refuses the read with the same head in its body.
fn head(at: SocketAddr, timing: Timing) -> Option<u64> {
    #[derive(Deserialize)]
    struct Head {
        head: u64,
    }
    let response = get(at, "/v0/beads/events?since=0&limit=1", timing)?;
    serde_json::from_reader::<_, Head>(response.body)
        .ok()
        .map(|read| read.head)
}

/// A response's status, and its body still to be read.
struct Response {
    status: u16,
    body: BufReader<TcpStream>,
}

/// Ask the bd serve at `at` for `path`.
///
/// HTTP/1.0, so the body comes unchunked and ends when the connection does.
fn get(at: SocketAddr, path: &str, timing: Timing) -> Option<Response> {
    let mut connection = TcpStream::connect_timeout(&at, timing.silent_after).ok()?;
    connection
        .set_read_timeout(Some(timing.silent_after))
        .ok()?;
    write!(connection, "GET {path} HTTP/1.0\r\nHost: {at}\r\n\r\n").ok()?;
    let mut body = BufReader::new(connection);
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if body.read_until(b'\n', &mut head).ok()? == 0 {
            return None;
        }
    }
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut response = httparse::Response::new(&mut headers);
    response.parse(&head).ok()?;
    Some(Response {
        status: response.code?,
        body,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Read};
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};

    use super::*;
    use std::time::Instant;

    const QUICKLY: Timing = Timing {
        silent_after: Duration::from_millis(300),
        again_after: Duration::from_millis(10),
    };

    /// Long enough for what is coming to have come, and short enough that
    /// what is not is a failure rather than a hang.
    const GIVING_UP: Duration = Duration::from_secs(10);

    const A_STREAM_OPENING: &str = "retry: 3000\n\n";

    /// A bd serve that answers each request as the test says, with a
    /// follower of its stream for arkham.
    struct Serve {
        listener: TcpListener,
        heard: Option<Receiver<Heard>>,
        answering: Option<TcpStream>,
    }

    impl Serve {
        fn followed() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a port is ours to take");
            listener
                .set_nonblocking(true)
                .expect("an accept is ours to give up on");
            let at = listener.local_addr().expect("the port is bound");
            let (report, heard) = mpsc::channel();
            thread::spawn(move || follow("arkham", at, &report, QUICKLY));
            Self {
                listener,
                heard: Some(heard),
                answering: None,
            }
        }

        /// The path the next request asks for, and that request's Host.
        fn asked_with_host(&mut self) -> (String, String) {
            let giving_up = Instant::now() + GIVING_UP;
            let connection = loop {
                match self.listener.accept() {
                    Ok((connection, _)) => break connection,
                    Err(why) if why.kind() == ErrorKind::WouldBlock => {
                        assert!(Instant::now() < giving_up, "nothing asked");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(why) => panic!("the follower's connection is accepted ({why})"),
                }
            };
            connection
                .set_nonblocking(false)
                .expect("the connection is ours to block on");
            let mut request = BufReader::new(
                connection
                    .try_clone()
                    .expect("the connection is ours to read"),
            );
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                request.read_line(&mut line).expect("the request comes");
                if line == "\r\n" {
                    break;
                }
                lines.push(line.trim_end().to_string());
            }
            self.answering = Some(connection);
            let path = lines[0]
                .strip_prefix("GET ")
                .and_then(|rest| rest.strip_suffix(" HTTP/1.0"))
                .unwrap_or_else(|| panic!("a GET over HTTP/1.0: {}", lines[0]))
                .to_string();
            let host = lines
                .iter()
                .find_map(|line| line.strip_prefix("Host: "))
                .expect("the request names its host")
                .to_string();
            (path, host)
        }

        fn asked(&mut self) -> String {
            self.asked_with_host().0
        }

        fn answers(&mut self, status: &str, body: &str) -> &mut Self {
            self.sends(&format!(
                "HTTP/1.0 {status}\r\nContent-Type: text/plain\r\n\r\n{body}"
            ))
        }

        fn answers_with_head(&mut self, head: u64) {
            assert_eq!(self.asked(), "/v0/beads/events?since=0&limit=1");
            self.answers("200 OK", &format!(r#"{{"head":{head},"records":[]}}"#));
            self.hangs_up();
        }

        fn sends(&mut self, text: &str) -> &mut Self {
            let answering = self.answering.as_mut().expect("a request to answer");
            answering
                .write_all(text.as_bytes())
                .expect("the follower reads what it asked for");
            self
        }

        fn hangs_up(&mut self) {
            self.answering = None;
        }

        fn hears(&self) -> Heard {
            self.heard
                .as_ref()
                .expect("the follower is heard")
                .recv_timeout(GIVING_UP)
                .expect("the follower says something")
        }

        fn hears_nothing_for(&self, quiet: Duration) {
            match self
                .heard
                .as_ref()
                .expect("the follower is heard")
                .recv_timeout(quiet)
            {
                Err(RecvTimeoutError::Timeout) => {}
                said => panic!("the follower said nothing, not {said:?}"),
            }
        }

        /// Following arkham's stream from `since`, opened.
        fn streaming_from(&mut self, since: u64) {
            assert_eq!(
                self.asked(),
                format!("/v0/beads/events:watch?since={since}")
            );
            self.answers("200 OK", A_STREAM_OPENING);
            assert_eq!(self.hears(), covered());
        }
    }

    /// Nothing is left to hear the follower, so the next thing it says ends
    /// it, whichever request it was making.
    impl Drop for Serve {
        fn drop(&mut self) {
            self.heard = None;
            self.answering = None;
            let giving_up = Instant::now() + Duration::from_secs(1);
            while Instant::now() < giving_up {
                if let Ok((mut connection, _)) = self.listener.accept() {
                    let _ = connection.set_nonblocking(false);
                    let mut request = [0; 1024];
                    let _ = connection.read(&mut request);
                    let _ = connection.write_all(b"HTTP/1.0 200 OK\r\n\r\n{\"head\":0}");
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn changed() -> Heard {
        Heard::Changed("arkham".to_string())
    }

    fn covered() -> Heard {
        Heard::Covered("arkham".to_string())
    }

    #[test]
    fn a_stream_is_followed_from_the_journals_head_after_one_full_read() {
        let mut serve = Serve::followed();

        let (_, host) = serve.asked_with_host();
        serve
            .answers("200 OK", r#"{"head":7,"records":[]}"#)
            .hangs_up();

        assert_eq!(serve.hears(), changed(), "the full read");
        assert_eq!(host, serve.listener.local_addr().unwrap().to_string());
        serve.streaming_from(7);
    }

    #[test]
    fn each_record_on_the_stream_says_the_project_changed() {
        let mut serve = Serve::followed();
        serve.answers_with_head(7);
        assert_eq!(serve.hears(), changed());
        serve.streaming_from(7);

        serve.sends("id: 8\ndata: {\"seq\":8}\n\n");
        assert_eq!(serve.hears(), changed());
        serve.sends("id: 9\ndata: {\"seq\":9}\n\nid: 10\ndata: {\"seq\":10}\n\n");
        assert_eq!(serve.hears(), changed());
        assert_eq!(serve.hears(), changed());
    }

    #[test]
    fn a_heartbeat_vouches_for_the_project() {
        let mut serve = Serve::followed();
        serve.answers_with_head(7);
        assert_eq!(serve.hears(), changed());
        serve.streaming_from(7);

        serve.sends(": heartbeat\n\n");

        assert_eq!(serve.hears(), covered());
    }

    #[test]
    fn a_dropped_stream_resumes_after_the_last_record_read_and_reads_nothing_in_full() {
        let mut serve = Serve::followed();
        serve.answers_with_head(7);
        assert_eq!(serve.hears(), changed());
        serve.streaming_from(7);
        serve.sends("id: 8\ndata: {\"seq\":8}\n\n");
        assert_eq!(serve.hears(), changed());

        serve.hangs_up();

        serve.streaming_from(8);
        serve.sends("id: 9\ndata: {\"seq\":9}\n\n");
        assert_eq!(serve.hears(), changed());
    }

    #[test]
    fn a_resume_the_journal_cannot_serve_reads_the_project_in_full_once_and_follows_from_its_head()
    {
        let mut serve = Serve::followed();
        serve.answers_with_head(7);
        assert_eq!(serve.hears(), changed());
        serve.streaming_from(7);
        serve.sends("id: 8\ndata: {\"seq\":8}\n\n");
        assert_eq!(serve.hears(), changed());
        serve.hangs_up();

        assert_eq!(serve.asked(), "/v0/beads/events:watch?since=8");
        serve
            .answers(
                "410 Gone",
                r#"{"code":"events_journal_truncated","since":8,"floor":20,"head":31}"#,
            )
            .hangs_up();
        serve.answers_with_head(31);

        assert_eq!(serve.hears(), changed(), "the full read");
        serve.streaming_from(31);
    }

    #[test]
    fn a_stream_the_journal_is_pruned_under_reads_the_project_in_full_once_and_follows_from_its_head(
    ) {
        let mut serve = Serve::followed();
        serve.answers_with_head(7);
        assert_eq!(serve.hears(), changed());
        serve.streaming_from(7);

        serve.sends("retry: 60000\n\nevent: truncated\ndata: {\"code\":\"events_journal_truncated\",\"head\":31}\n\n");
        serve.hangs_up();
        serve.answers_with_head(31);

        assert_eq!(serve.hears(), changed(), "the full read");
        serve.streaming_from(31);
    }

    #[test]
    fn a_stream_silent_past_its_heartbeat_vouches_for_nothing_and_is_resumed() {
        let mut serve = Serve::followed();
        serve.answers_with_head(7);
        assert_eq!(serve.hears(), changed());
        serve.streaming_from(7);
        serve.sends("id: 8\ndata: {\"seq\":8}\n\n");
        assert_eq!(serve.hears(), changed());

        serve.hears_nothing_for(QUICKLY.silent_after);

        serve.streaming_from(8);
    }

    #[test]
    fn a_bd_serve_that_will_not_answer_vouches_for_nothing_and_is_asked_again() {
        let mut serve = Serve::followed();
        serve.asked();
        serve
            .answers("503 Service Unavailable", r#"{"code":"unavailable"}"#)
            .hangs_up();

        serve.answers_with_head(7);

        assert_eq!(serve.hears(), changed(), "the first word is the full read");
    }
}

//! `bdi web` answers a browser with the forest a tracker holds, and each
//! bead's own page at the address its project and id compose.
//!
//! The cases run the binary, because what is under test is the process a
//! supervisor starts and the port it answers on.

mod terminal;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use terminal::shims::ShimmedTracker;
use terminal::{
    a_home_naming_one_project, a_home_naming_one_project_settled, die_with, THE_DESCRIBED_SUBTREE,
};

/// Long enough for a read that is coming to have come, and short enough
/// that one that is not is a failure rather than a hang.
const GIVING_UP: Duration = Duration::from_secs(10);

/// The capture's root, and a bead beneath it, each with its title.
const THE_ROOT: (&str, &str) = (
    "dun-0tp",
    "Every outside program is optional: one seam per capability, and a setup that lacks one runs without it",
);
const A_BEAD_BENEATH: (&str, &str) = (
    "dun-0tp.7",
    "phrase.rs is one file every screen change edits",
);

/// A running `bdi web`, killed and reaped if the test ends without stopping
/// it, which a panic does.
struct Web(Child);

impl Drop for Web {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `bdi web` over `tracker`, on a port the system picks, answering for the
/// host names `allowing` as well as its address, and the address it says it
/// is answering on.
fn bdi_web(home: &Path, tracker: &ShimmedTracker, allowing: &[&str]) -> (Web, String) {
    let spawned_by = std::process::id();
    let mut command = Command::new(env!("CARGO_BIN_EXE_bdi"));
    command
        .args(["web", "--listen", "127.0.0.1:0"])
        .args(allowing.iter().flat_map(|host| ["--allow-host", host]))
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .env("XDG_RUNTIME_DIR", home)
        .envs(tracker.environment())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Dropping a `Child` leaves its process running, and a test binary that
    // is killed runs no `Drop`, so the kernel is asked to end it with its
    // spawner.
    unsafe { command.pre_exec(move || die_with(spawned_by)) };
    let mut child = command.spawn().expect("bdi web starts");
    let said = first_line(child.stdout.take().expect("its stdout"));
    let address = said
        .trim_end()
        .rsplit_once("http://")
        .map(|(_, address)| address.to_string())
        .unwrap_or_else(|| panic!("bdi web said where it answers: {said:?}"));
    (Web(child), address)
}

fn first_line(stdout: ChildStdout) -> String {
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("bdi web says where it answers");
    line
}

/// What `address` answers a GET of `path` sent to it by its address with:
/// its status line and its body.
fn get(address: &str, path: &str) -> (String, String) {
    get_from(address, address, path)
}

/// What `address` answers a GET of `path` sent to the host `host` with: its
/// status line and its body.
fn get_from(address: &str, host: &str, path: &str) -> (String, String) {
    let mut stream = TcpStream::connect(address).expect("bdi web takes the connection");
    stream
        .set_read_timeout(Some(GIVING_UP))
        .expect("a read timeout");
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").expect("the request is sent");
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .expect("bdi web answers and closes");
    let (head, body) = answer.split_once("\r\n\r\n").expect("a head and a body");
    let status = head.lines().next().unwrap_or_default().to_string();
    (status, body.to_string())
}

/// `bdi web` over the described subtree, answering for the host names
/// `allowing`, once it has read the tracker.
fn a_web_over_the_tracker(named: &str, allowing: &[&str]) -> (Web, String) {
    let home = a_home_naming_one_project(named);
    let (web, address, _) = serving(&home, allowing);
    (web, address)
}

/// `bdi web` in `home` over the described subtree, answering for the host
/// names `allowing`, once it has read the tracker, and the tracker, which the
/// test may change.
fn serving(home: &Path, allowing: &[&str]) -> (Web, String, ShimmedTracker) {
    let tracker = ShimmedTracker::beside(home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let (web, address) = bdi_web(home, &tracker, allowing);
    let giving_up = Instant::now() + GIVING_UP;
    while get(&address, "/healthz").0 != "HTTP/1.1 200 OK" {
        assert!(Instant::now() < giving_up, "bdi web never read the tracker");
        std::thread::sleep(Duration::from_millis(20));
    }
    (web, address, tracker)
}

/// A page elsewhere that points a name of its own at the listener sends that
/// name as the host, so only a name the listener was given is answered.
#[test]
fn only_a_request_sent_to_a_host_it_was_given_is_answered() {
    let (_web, address) = a_web_over_the_tracker("web-hosts", &["beads.example.invalid"]);
    let (status, _) = get_from(&address, "dunwich.invalid", "/");
    assert_eq!(status, "HTTP/1.1 421 Misdirected Request");
    let (status, body) = get_from(&address, "beads.example.invalid:8080", "/");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(body.contains(THE_ROOT.1), "{body}");
}

#[test]
fn the_forest_a_tracker_holds_is_answered_as_a_page() {
    let (_web, address) = a_web_over_the_tracker("web-forest", &[]);
    let (status, body) = get(&address, "/");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(body.starts_with("<!doctype html>"), "{body}");
    assert!(
        body.contains(THE_ROOT.0) && body.contains(THE_ROOT.1),
        "{body}"
    );
    assert!(
        body.contains("href=\"/arkham/dun-0tp?at=arkham%3Adun-0tp#shown\""),
        "the root's row opens it: {body}"
    );
}

#[test]
fn a_beads_address_answers_with_that_bead_shown() {
    let (_web, address) = a_web_over_the_tracker("web-bead", &[]);
    let (status, body) = get(&address, &format!("/arkham/{}", A_BEAD_BENEATH.0));
    assert_eq!(status, "HTTP/1.1 200 OK");
    let aside = &body[body.find("<aside>").expect("the bead view is drawn")..];
    assert!(aside.contains(A_BEAD_BENEATH.1), "{aside}");
    // The root rests shut, so the page opened it to reach the bead, and the
    // link keeps it open.
    assert!(
        aside.contains("href=\"/arkham/dun-0tp?open=b%3Aarkham%3Adun-0tp#shown\""),
        "the bead's parent opens it: {aside}"
    );

    let (status, _) = get(&address, "/arkham/dun-zzz");
    assert_eq!(status, "HTTP/1.1 404 Not Found");
}

/// A page held open is sent each change the server holds down the one
/// connection its script opens, and asks for nothing else.
#[test]
fn an_open_page_is_sent_each_change_down_the_one_connection_it_holds() {
    const RETITLED: &str = "A title the tracker gave the root after the page was drawn";
    let home = a_home_naming_one_project_settled("web-live", "[tui]\nrefresh_seconds = 1\n");
    let (_web, address, tracker) = serving(&home, &[]);
    let (_, page) = get(&address, "/");
    let change = page
        .split_once("data-change=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(change, _)| change.to_string())
        .unwrap_or_else(|| panic!("the page names the change it was drawn from: {page}"));

    let mut stream = TcpStream::connect(&address).expect("bdi web takes the connection");
    stream
        .set_read_timeout(Some(GIVING_UP))
        .expect("a read timeout");
    write!(
        stream,
        "GET /?since={change} HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n"
    )
    .expect("the request is sent");
    let mut events = BufReader::new(stream);
    let mut head = String::new();
    while !head.ends_with("\r\n\r\n") {
        assert!(
            events.read_line(&mut head).expect("a head") > 0,
            "the head ends: {head}"
        );
    }
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(
        head.contains("Content-Type: text/event-stream\r\n"),
        "{head}"
    );

    tracker.holds(&THE_DESCRIBED_SUBTREE.replace(THE_ROOT.1, RETITLED));
    let giving_up = Instant::now() + GIVING_UP;
    let mut held: Option<Vec<String>> = None;
    loop {
        assert!(Instant::now() < giving_up, "the change never arrived");
        let (kind, data) = next_event(&mut events);
        held = Some(match kind.as_str() {
            "page" => data,
            "patch" => patched(held.expect("a patch follows a page"), &data),
            _ => continue,
        });
        let now = held.as_ref().expect("a page").join("\n");
        if now.contains(RETITLED) {
            assert!(now.starts_with("<!doctype html>"), "{now}");
            assert!(!now.contains(THE_ROOT.1), "{now}");
            break;
        }
    }
}

/// The next event `events` sends: its kind, and its data a line at a time.
fn next_event(events: &mut impl BufRead) -> (String, Vec<String>) {
    let (mut kind, mut data) = (String::new(), Vec::new());
    loop {
        let mut line = String::new();
        assert!(
            events.read_line(&mut line).expect("an event") > 0,
            "the stream stays open"
        );
        let line = line.trim_end_matches('\n');
        if line.is_empty() {
            if !data.is_empty() {
                return (kind, data);
            }
            continue;
        }
        if let Some(said) = line.strip_prefix("event: ") {
            kind = said.to_string();
        } else if let Some(said) = line.strip_prefix("data:") {
            data.push(said.strip_prefix(' ').unwrap_or(said).to_string());
        }
    }
}

/// `page` with each hunk of `patch` applied in turn, as the page's script
/// applies them: a line saying where, how many lines go and how many come,
/// then the lines that come.
fn patched(mut page: Vec<String>, patch: &[String]) -> Vec<String> {
    let mut lines = patch.iter();
    while let Some(hunk) = lines.next() {
        let counts: Vec<usize> = hunk
            .split(' ')
            .map(|count| count.parse().expect("a count"))
            .collect();
        let [at, going, coming] = counts[..] else {
            panic!("a hunk names three counts: {hunk}");
        };
        let came: Vec<String> = lines.by_ref().take(coming).cloned().collect();
        page.splice(at..at + going, came);
    }
    page
}

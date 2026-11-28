//! A `bdi web` of the test's own, and the GETs a test sends it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use super::die_with;
use super::shims::ShimmedTracker;

/// Long enough for an answer that is coming to have come, and short enough
/// that one that is not is a failure rather than a hang.
const GIVING_UP: Duration = Duration::from_secs(10);

/// A running `bdi web`, killed and reaped if the test ends without stopping
/// it, which a panic does.
pub struct Web(pub Child);

impl Drop for Web {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `bdi web` over `tracker` given `args`, on a port the system picks, and
/// the address it says it is answering on. A machine with no runtime
/// directory has no socket a producer or a watcher could be found on unless
/// the config names one.
pub fn bdi_web(
    home: &Path,
    tracker: &ShimmedTracker,
    args: &[&str],
    runtime_dir: Option<&Path>,
    exported: &[(&str, &str)],
) -> (Web, String) {
    let spawned_by = std::process::id();
    let mut command = Command::new(env!("CARGO_BIN_EXE_bdi"));
    command
        .args(["web", "--listen", "127.0.0.1:0"])
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .env_remove("XDG_RUNTIME_DIR")
        .envs(runtime_dir.map(|dir| ("XDG_RUNTIME_DIR", dir)))
        .envs(tracker.environment())
        .envs(exported.iter().copied())
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
pub fn get(address: &str, path: &str) -> (String, String) {
    get_from(address, address, path)
}

/// What `address` answers a GET of `path` sent to the host `host` with: its
/// status line and its body.
pub fn get_from(address: &str, host: &str, path: &str) -> (String, String) {
    let (head, body) = fetched(address, host, path, "");
    let status = head.lines().next().unwrap_or_default().to_string();
    (status, String::from_utf8(body).expect("a page is text"))
}

/// What `address` answers a GET of `path` sent to the host `host` with the
/// header lines `offering`: its head and its body as it came on the wire.
pub fn fetched(address: &str, host: &str, path: &str, offering: &str) -> (String, Vec<u8>) {
    let mut stream = TcpStream::connect(address).expect("bdi web takes the connection");
    stream
        .set_read_timeout(Some(GIVING_UP))
        .expect("a read timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\n{offering}\r\n"
    )
    .expect("the request is sent");
    let mut answer = Vec::new();
    stream
        .read_to_end(&mut answer)
        .expect("bdi web answers and closes");
    let end = answer
        .windows(4)
        .position(|four| four == b"\r\n\r\n")
        .expect("a head and a body");
    let head = String::from_utf8(answer[..end + 2].to_vec()).expect("a head is text");
    (head, answer[end + 4..].to_vec())
}

/// Wait until the `bdi web` at `address` has read the tracker.
pub fn until_it_has_read(address: &str) {
    let giving_up = Instant::now() + GIVING_UP;
    while get(address, "/healthz").0 != "HTTP/1.1 200 OK" {
        assert!(Instant::now() < giving_up, "bdi web never read the tracker");
        std::thread::sleep(Duration::from_millis(20));
    }
}

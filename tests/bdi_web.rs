//! `bdi web` answers a browser with the forest a tracker holds, and each
//! bead's own page at the address its project and id compose.
//!
//! The cases run the binary, because what is under test is the process a
//! supervisor starts and the port it answers on.

#[cfg(target_os = "linux")]
mod browser;
mod terminal;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use terminal::shims::ShimmedTracker;
use terminal::{
    a_home_naming_one_project, a_home_naming_one_project_settled, die_with, EACH_VIEW, THE_ARCHIVE,
    THE_DESCRIBED_SUBTREE, TWO_VIEWS,
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
    let (head, body) = fetched(address, host, path, "");
    let status = head.lines().next().unwrap_or_default().to_string();
    (status, String::from_utf8(body).expect("a page is text"))
}

/// What `address` answers a GET of `path` sent to the host `host` with the
/// header lines `offering`: its head and its body as it came on the wire.
fn fetched(address: &str, host: &str, path: &str, offering: &str) -> (String, Vec<u8>) {
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

/// `bdi web` over the described subtree, answering for the host names
/// `allowing`, once it has read the tracker.
fn a_web_over_the_tracker(named: &str, allowing: &[&str]) -> (Web, String) {
    a_web_over(
        &a_home_naming_one_project(named),
        THE_DESCRIBED_SUBTREE,
        allowing,
    )
}

/// `bdi web` in `home`, over a tracker holding `beads` and answering for
/// the host names `allowing`, once it has read the tracker.
fn a_web_over(home: &Path, beads: &str, allowing: &[&str]) -> (Web, String) {
    let (web, address, _) = serving(home, beads, allowing);
    (web, address)
}

/// `bdi web` in `home`, over a tracker holding `beads` and answering for
/// the host names `allowing`, once it has read the tracker, and the tracker,
/// which the test may change.
fn serving(home: &Path, beads: &str, allowing: &[&str]) -> (Web, String, ShimmedTracker) {
    let tracker = ShimmedTracker::beside(home);
    tracker.holds(beads);
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
    an_open_page_kept_current("web-live", "");
}

/// A reader offering gzip is sent the page and its changes gzipped, and a
/// change still reaches it without waiting on the one after.
#[test]
fn a_reader_offering_gzip_is_sent_the_page_and_its_changes_gzipped() {
    an_open_page_kept_current("web-live-gzipped", "Accept-Encoding: gzip, deflate, br\r\n");
}

/// A page fetched and then kept current by a reader sending the header
/// lines `offering`, which says gzip where it offers it, in a home `named`.
fn an_open_page_kept_current(named: &str, offering: &str) {
    const GZIPPED: &str = "Content-Encoding: gzip\r\n";
    const RETITLED: &str = "A title the tracker gave the root after the page was drawn";
    let gzipped = offering.contains("gzip");
    let home = a_home_naming_one_project_settled(named, "[tui]\nrefresh_seconds = 1\n");
    let (_web, address, tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &[]);
    let (head, page) = fetched(&address, &address, "/", offering);
    assert_eq!(head.contains(GZIPPED), gzipped, "{head}");
    let page = if gzipped {
        let mut unzipped = String::new();
        flate2::read::GzDecoder::new(&page[..])
            .read_to_string(&mut unzipped)
            .expect("the page decodes");
        unzipped
    } else {
        String::from_utf8(page).expect("a page is text")
    };
    let version = page
        .split_once("data-version=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(version, _)| version.to_string())
        .unwrap_or_else(|| panic!("the page names its version: {page}"));

    let mut stream = TcpStream::connect(&address).expect("bdi web takes the connection");
    stream
        .set_read_timeout(Some(GIVING_UP))
        .expect("a read timeout");
    write!(
        stream,
        "GET /?since={version} HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n{offering}\r\n"
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
    assert_eq!(head.contains(GZIPPED), gzipped, "{head}");
    let mut events: Box<dyn BufRead> = if gzipped {
        Box::new(BufReader::new(flate2::bufread::GzDecoder::new(events)))
    } else {
        Box::new(events)
    };

    tracker.holds(&THE_DESCRIBED_SUBTREE.replace(THE_ROOT.1, RETITLED));
    let giving_up = Instant::now() + GIVING_UP;
    let mut held: Vec<String> = page.split('\n').map(str::to_string).collect();
    loop {
        assert!(Instant::now() < giving_up, "the change never arrived");
        let (kind, data) = next_event(&mut events);
        assert_eq!(
            kind, "patch",
            "a reader holding the page is sent patches to it"
        );
        held = patched(held, &data);
        let now = held.join("\n");
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

/// A change to one bead, pushed to a page open on another, redraws that
/// bead's line alone. Every element of the page is the one it was, each other
/// bead's line and the open bead say what they said, and the window and the
/// bead view are scrolled where the reader left them. The project's heading
/// is left out, because it says when the project was read, and a change is
/// a new read.
#[cfg(target_os = "linux")]
#[test]
fn a_change_pushed_to_an_open_page_redraws_only_the_bead_it_changed() {
    const RETITLED: &str = "A title the tracker gave a sibling after the page was drawn";
    let home = a_home_naming_one_project_settled("web-redraw", "[tui]\nrefresh_seconds = 1\n");
    let (_web, address, tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &[]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", A_BEAD_BENEATH.0),
        1200,
        240,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('aside')",
        GIVING_UP,
    );
    let scrolled = page.evaluate(
        r#"scrollTo(0, document.scrollingElement.scrollHeight);
        document.querySelector("aside").scrollTop = 40;
        window.before = [...document.body.querySelectorAll("*")];
        window.BEADS = ".line[id]:not(.heading), aside";
        window.CHANGED = ".line[id$='/arkham:dun-0tp.9']";
        window.unchanged = () => [...document.querySelectorAll(BEADS)]
            .filter((at) => !at.matches(CHANGED))
            .map((at) => at.cloneNode(true))
            .map((at) => (at.querySelector(".rails")?.remove(), at.outerHTML));
        window.lines = unchanged();
        window.field = document.querySelector("input[type=search]");
        field.value = "typed but not sent";
        window.focused = document.querySelector(CHANGED + " a[href]");
        focused.focus();
        [scrollY, document.querySelector("aside").scrollTop]"#,
    );
    let [window_at, bead_at] =
        [&scrolled[0], &scrolled[1]].map(|at| at.as_f64().expect("a scroll"));
    assert!(
        window_at > 0.0 && bead_at > 0.0,
        "the page scrolls: {scrolled}"
    );

    let mut beads: serde_json::Value =
        serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("the fixture is JSON");
    let sibling = beads
        .as_array_mut()
        .and_then(|beads| beads.iter_mut().find(|bead| bead["id"] == "dun-0tp.9"))
        .expect("the fixture holds the sibling");
    sibling["title"] = RETITLED.into();
    // A higher priority moves its line above its siblings'.
    sibling["priority"] = 1.into();
    tracker.holds(&beads.to_string());
    page.until(
        &format!("document.querySelector(CHANGED).textContent.includes({RETITLED:?})"),
        GIVING_UP,
    );
    assert_eq!(
        page.evaluate("before.filter((at) => !at.isConnected).map((at) => at.outerHTML)"),
        serde_json::json!([]),
        "every element is still the page's own"
    );
    assert_eq!(
        page.evaluate("unchanged()"),
        page.evaluate("lines"),
        "only the changed bead's line says something new, apart from the rails its move redrew"
    );
    assert_eq!(
        page.evaluate("document.activeElement === focused"),
        serde_json::json!(true),
        "the link the reader focused keeps the focus as its line moves"
    );
    assert_eq!(
        page.evaluate("field.isConnected && field.value"),
        serde_json::json!("typed but not sent"),
        "the find field keeps what the reader typed"
    );
    assert_eq!(
        page.evaluate("[scrollY, document.querySelector('aside').scrollTop]"),
        scrolled,
        "the window and the bead view stay where the reader scrolled them"
    );
}

/// Opening a page sends its content once: the browser fetches the page,
/// and its script is sent no page of its own while the server reads the
/// tracker again and again. A page hidden while a bead changes is sent the
/// page once more when it is shown again, and shows the change.
#[cfg(target_os = "linux")]
#[test]
fn opening_a_page_and_returning_to_it_after_a_change_each_send_it_once() {
    const RETITLED: &str = "A title the tracker gave the root while the page was hidden";
    let home = a_home_naming_one_project_settled("web-once", "[tui]\nrefresh_seconds = 1\n");
    let (_web, address, tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &[]);
    let relay = Relay::to(&address);
    let mut page = browser::Browser::open(&home, &format!("http://{}/", relay.address), 1200, 600);
    page.until("document.readyState === 'complete'", GIVING_UP);
    relay.until_asked(2);
    // Each second's read draws the page twice, as the read goes out and
    // as it comes back.
    std::thread::sleep(Duration::from_secs(3));
    let opened = relay.answered();
    let [(fetched, page_bytes), (listened, stream_bytes)] = &opened[..] else {
        panic!("the page asked for itself and then for its changes alone: {opened:?}");
    };
    assert!(!fetched.contains("since="), "{fetched}");
    assert!(
        listened.contains("since="),
        "the script holds the page: {listened}"
    );
    assert!(
        stream_bytes * 4 < *page_bytes,
        "opening sent the page's {page_bytes} bytes, then {stream_bytes} more down its stream"
    );

    page.evaluate(
        r#"Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
        document.dispatchEvent(new Event("visibilitychange"))"#,
    );
    tracker.holds(&THE_DESCRIBED_SUBTREE.replace(THE_ROOT.1, RETITLED));
    let giving_up = Instant::now() + GIVING_UP;
    while !get(&address, "/").1.contains(RETITLED) {
        assert!(Instant::now() < giving_up, "bdi web never read the change");
        std::thread::sleep(Duration::from_millis(50));
    }
    page.evaluate(
        r#"Object.defineProperty(document, "hidden", { configurable: true, get: () => false });
        document.dispatchEvent(new Event("visibilitychange"))"#,
    );
    page.until(
        &format!("document.body.textContent.includes({RETITLED:?})"),
        GIVING_UP,
    );
    std::thread::sleep(Duration::from_secs(3));
    let returned = relay.answered();
    let [_, _, (listened, stream_bytes)] = &returned[..] else {
        panic!("the shown page asked for its changes alone: {returned:?}");
    };
    assert!(listened.contains("since="), "{listened}");
    assert!(
        stream_bytes * 2 < page_bytes * 3,
        "returning sent {stream_bytes} bytes for a page of {page_bytes}"
    );
}

/// A relay between a browser and `bdi web`, which keeps the first line of
/// each request a connection carries and counts the bytes `bdi web` sends
/// back down it.
#[cfg(target_os = "linux")]
struct Relay {
    address: String,
    connections: std::sync::Arc<std::sync::Mutex<Vec<(String, usize)>>>,
}

#[cfg(target_os = "linux")]
impl Relay {
    fn to(web: &str) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a listener");
        let address = listener.local_addr().expect("an address").to_string();
        let connections: std::sync::Arc<std::sync::Mutex<Vec<(String, usize)>>> =
            std::sync::Arc::default();
        let kept = std::sync::Arc::clone(&connections);
        let web = web.to_string();
        std::thread::spawn(move || {
            for browser in listener.incoming() {
                let Ok(browser) = browser else { return };
                let to_web = TcpStream::connect(&web).expect("bdi web takes the connection");
                let at = {
                    let mut connections = kept.lock().expect("the count");
                    connections.push((String::new(), 0));
                    connections.len() - 1
                };
                let (asked, answered) =
                    (std::sync::Arc::clone(&kept), std::sync::Arc::clone(&kept));
                let (mut from_browser, mut to_browser) =
                    (browser.try_clone().expect("the browser's side"), browser);
                let (mut from_web, mut web_side) =
                    (to_web.try_clone().expect("bdi web's side"), to_web);
                std::thread::spawn(move || {
                    let mut said = [0; 1 << 16];
                    while let Ok(got @ 1..) = from_browser.read(&mut said) {
                        let first = &mut asked.lock().expect("the count")[at].0;
                        if first.is_empty() {
                            let line = String::from_utf8_lossy(&said[..got]);
                            *first = line.lines().next().unwrap_or_default().to_string();
                        }
                        if web_side.write_all(&said[..got]).is_err() {
                            break;
                        }
                    }
                    let _ = web_side.shutdown(std::net::Shutdown::Write);
                });
                std::thread::spawn(move || {
                    let mut said = [0; 1 << 16];
                    while let Ok(got @ 1..) = from_web.read(&mut said) {
                        answered.lock().expect("the count")[at].1 += got;
                        if to_browser.write_all(&said[..got]).is_err() {
                            break;
                        }
                    }
                    let _ = to_browser.shutdown(std::net::Shutdown::Write);
                });
            }
        });
        Self {
            address,
            connections,
        }
    }

    /// Each connection the browser made other than for its favicon: its
    /// request's first line and the bytes `bdi web` answered it with.
    fn answered(&self) -> Vec<(String, usize)> {
        self.connections
            .lock()
            .expect("the count")
            .iter()
            .filter(|(asked, _)| !asked.is_empty() && !asked.contains("favicon"))
            .cloned()
            .collect()
    }

    fn until_asked(&self, connections: usize) {
        let giving_up = Instant::now() + GIVING_UP;
        while self.answered().len() < connections {
            assert!(
                Instant::now() < giving_up,
                "asked only {:?}",
                self.answered()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Where the entry of `page`'s list menu that says `said` goes.
fn menu_entry(page: &str, said: &str) -> String {
    let menu = &page[page.find("<details class=\"lists\">").expect("a list menu")..];
    let menu = &menu[..menu.find("</details>").expect("the menu ends")];
    let end = menu
        .find(&format!(">{said}<span class=\"count\">"))
        .unwrap_or_else(|| panic!("the menu offers {said}: {menu}"));
    let href = &menu[menu[..end]
        .rfind("href=\"")
        .expect("the entry goes somewhere")
        + 6..end];
    href[..href.find('"').expect("the address ends")].replace("&amp;", "&")
}

/// Each view the config names is an entry of the page's menu, and picking
/// it lists the beads its conditions select and none of the others, as `v`
/// does in the terminal.
#[test]
fn each_view_picked_from_the_menu_lists_only_what_it_selects() {
    let home = a_home_naming_one_project_settled("web-views", TWO_VIEWS);
    let (_web, address) = a_web_over(&home, THE_ARCHIVE, &[]);
    let (_, trees) = get(&address, "/");

    for (view, heading, listed, left_out) in EACH_VIEW {
        let (status, page) = get(&address, &menu_entry(&trees, view));
        assert_eq!(status, "HTTP/1.1 200 OK");
        for said in [heading, listed] {
            assert!(page.contains(said), "{view} does not draw {said}: {page}");
        }
        for bead in left_out {
            assert!(
                !page.contains(bead),
                "{view} lists {bead}, which it does not select: {page}"
            );
        }
    }
}

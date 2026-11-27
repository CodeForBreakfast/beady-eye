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

/// `bdi web` over `tracker` given `args`, on a port the system picks, and
/// the address it says it is answering on. A machine with no runtime
/// directory has no socket a producer or a watcher could be found on unless
/// the config names one.
fn bdi_web(
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

/// `bdi web` over the described subtree given `args`, once it has read the
/// tracker.
fn a_web_over_the_tracker(named: &str, args: &[&str]) -> (Web, String) {
    a_web_over(
        &a_home_naming_one_project(named),
        THE_DESCRIBED_SUBTREE,
        args,
    )
}

/// `bdi web` in `home` given `args`, over a tracker holding `beads`, once
/// it has read the tracker.
fn a_web_over(home: &Path, beads: &str, args: &[&str]) -> (Web, String) {
    let (web, address, _) = serving(home, beads, args);
    (web, address)
}

/// `bdi web` in `home` given `args`, over a tracker holding `beads`, once
/// it has read the tracker, and the tracker, which the test may change.
fn serving(home: &Path, beads: &str, args: &[&str]) -> (Web, String, ShimmedTracker) {
    let tracker = ShimmedTracker::beside(home);
    tracker.holds(beads);
    let (web, address) = bdi_web(home, &tracker, args, Some(home), &[]);
    until_it_has_read(&address);
    (web, address, tracker)
}

/// Wait until the `bdi web` at `address` has read the tracker.
fn until_it_has_read(address: &str) {
    let giving_up = Instant::now() + GIVING_UP;
    while get(address, "/healthz").0 != "HTTP/1.1 200 OK" {
        assert!(Instant::now() < giving_up, "bdi web never read the tracker");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A server with nowhere a producer or a watcher could be found, such as a
/// container, still keeps the page current by polling, so it stays up.
#[test]
fn a_web_with_no_socket_to_hear_on_stays_up_and_answering() {
    let home = a_home_naming_one_project("web-no-socket");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let (mut web, address) = bdi_web(&home, &tracker, &[], None, &[]);
    until_it_has_read(&address);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        web.0.try_wait().expect("bdi web can be asked").is_none(),
        "bdi web stopped once it had read"
    );
    assert_eq!(get(&address, "/").0, "HTTP/1.1 200 OK");
}

/// A page elsewhere that points a name of its own at the listener sends that
/// name as the host, so only a name the listener was given is answered.
#[test]
fn only_a_request_sent_to_a_host_it_was_given_is_answered() {
    let (_web, address) =
        a_web_over_the_tracker("web-hosts", &["--allow-host", "beads.example.invalid"]);
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

/// A second project, whose tracker cannot be read, which the page says
/// under its name wherever `bdi web` reads it.
const A_SECOND_PROJECT: &str = "[[projects]]\nname = \"kadath\"\npath = \"/nonexistent/kadath\"\n";

#[test]
fn a_page_draws_only_the_projects_named_with_project() {
    let home = a_home_naming_one_project_settled("web-projects", A_SECOND_PROJECT);
    let (_web, address) = a_web_over(&home, THE_DESCRIBED_SUBTREE, &[]);
    let (_, every) = get(&address, "/");
    assert!(every.contains("kadath"), "{every}");
    drop(_web);
    let (_web, address) = a_web_over(&home, THE_DESCRIBED_SUBTREE, &["--project", "arkham"]);
    let (status, named) = get(&address, "/");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(named.contains(THE_ROOT.1), "{named}");
    assert!(!named.contains("kadath"), "{named}");
}

#[test]
fn under_ready_a_bare_address_opens_in_the_ready_list() {
    let (_web, address) = a_web_over_the_tracker("web-ready", &["--ready"]);
    let (status, body) = get(&address, "/");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(body.contains("<a class=\"ready on\""), "{body}");
    let (_, trees) = get(&address, "/?ready=0");
    assert!(trees.contains("<a class=\"ready\""), "{trees}");
}

#[test]
fn a_bead_named_on_the_command_line_roots_a_bare_address_at_it() {
    let named = format!("arkham:{}", A_BEAD_BENEATH.0);
    let (_web, address) = a_web_over_the_tracker("web-root", &[&named]);
    let (status, body) = get(&address, "/");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(body.contains("<p class=\"rooted\">"), "{body}");
    let (_, whole) = get(&address, "/?root=");
    assert!(!whole.contains("<p class=\"rooted\">"), "{whole}");
}

#[test]
fn a_beads_address_answers_with_that_bead_shown() {
    let (_web, address) = a_web_over_the_tracker("web-bead", &[]);
    let (status, body) = get(&address, &format!("/arkham/{}", A_BEAD_BENEATH.0));
    assert_eq!(status, "HTTP/1.1 200 OK");
    let aside = &body[body.find("<aside").expect("the bead view is drawn")..];
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
    let (mut page, page_bytes) = opened_once(&home, &relay, "/");

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
        stream_bytes * 4 < page_bytes,
        "returning sent {stream_bytes} bytes for a page of {page_bytes}"
    );
}

/// A page from a run that takes writes is sent once too, though each of its
/// forms carries a token that a later run will not take.
#[cfg(target_os = "linux")]
#[test]
fn opening_a_page_that_takes_writes_sends_it_once() {
    let home = a_home_naming_one_project_settled(
        "web-once-writes",
        &format!("[tui]\nrefresh_seconds = 1\n{WRITES_AS_CARTER}"),
    );
    let (_web, address, _tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &["--writes"]);
    let relay = Relay::to(&address);
    let (mut page, _) = opened_once(&home, &relay, "/arkham/dun-0tp");
    assert_eq!(
        page.evaluate(
            "[...document.querySelectorAll('details[name=write]')].map((entry) => entry.id)"
        ),
        serde_json::json!([
            "write-priority",
            "write-comment",
            "write-close",
            "write-defer"
        ]),
        "the page is the bead's, with an entry for each write it offers an open bead"
    );
}

/// Each write's entry on a writable bead's page opens from the keyboard:
/// by the key the terminal makes the write with, as soon as the page opens,
/// or by Enter on its button, and either moves to its first control: the
/// field it is typed in, or a deferral's first day to pick. The key held
/// down types nothing into the field, and Escape held down shuts the entry
/// and goes no further. A
/// letter typed while the focus is in the forest, or nowhere, opens
/// nothing. Escape in the empty field shuts
/// the entry and goes back to its button, and in a field holding words it
/// does nothing, so no words are lost. Ctrl+Enter makes the write a text
/// area is typed for.
#[cfg(target_os = "linux")]
#[test]
fn a_writable_beads_page_makes_a_write_from_the_keyboard() {
    let home = a_home_naming_one_project_settled("web-write-by-key", WRITES_AS_CARTER);
    let tracker = a_tracker_taking_the_comment(&home);
    let (_web, address) = bdi_web(&home, &tracker, &["--writes"], Some(&home), &[]);
    until_it_has_read(&address);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", THE_ROOT.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-comment')",
        GIVING_UP,
    );
    let in_the_comment = "document.querySelector('#write-comment').open \
         && document.activeElement === document.querySelector('#write-comment textarea')";
    let on_its_button = "!document.querySelector('#write-comment').open \
         && document.activeElement === document.querySelector('#write-comment>summary')";

    page.press("m", false);
    page.until(in_the_comment, GIVING_UP);
    page.press("Escape", false);
    page.until(on_its_button, GIVING_UP);
    page.press("Enter", false);
    page.until(in_the_comment, GIVING_UP);

    page.type_text("the gate is shut");
    page.press("Escape", false);
    assert_eq!(
        page.evaluate(&format!(
            "[{in_the_comment}, document.activeElement.value, location.pathname]"
        )),
        serde_json::json!([true, "the gate is shut", format!("/arkham/{}", THE_ROOT.0)]),
        "Escape leaves words typed where they are"
    );
    page.press("Enter", true);
    let giving_up = Instant::now() + GIVING_UP;
    while !tracker
        .acted_calls()
        .contains(&format!("carter {COMMENTED}"))
    {
        assert!(
            Instant::now() < giving_up,
            "Ctrl+Enter never made the comment: {:?}",
            tracker.acted_calls()
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-comment') \
         && !document.querySelector('#write-comment textarea').value",
        GIVING_UP,
    );
    page.press("m", false);
    page.until(in_the_comment, GIVING_UP);
    page.hold("Escape");
    page.until(on_its_button, GIVING_UP);
    assert_eq!(
        page.evaluate("location.pathname"),
        serde_json::json!(format!("/arkham/{}", THE_ROOT.0)),
        "Escape held down shuts the entry and stays on the bead"
    );
    page.hold("m");
    page.until(in_the_comment, GIVING_UP);
    assert_eq!(
        page.evaluate("document.activeElement.value"),
        serde_json::json!(""),
        "the key held down to open the comment types nothing into it"
    );
    page.press("Escape", false);
    page.press("z", false);
    page.until(
        "document.querySelector('#write-defer').open \
         && document.activeElement === document.querySelector('#write-defer .pick')",
        GIVING_UP,
    );
    page.press("Escape", false);
    for (focus, said) in [
        (
            "document.querySelector('.forest a[href]').focus()",
            "in the forest",
        ),
        ("document.activeElement.blur()", "with the focus nowhere"),
    ] {
        page.evaluate(focus);
        page.press("m", false);
        assert_eq!(
            page.evaluate("document.querySelector('details[name=write][open]')"),
            serde_json::Value::Null,
            "a letter typed {said} opens no write"
        );
    }
}

/// Holding Enter on the button of a write made with no words opens its
/// entry and moves to the button that makes it, and writes nothing: a
/// repeated Enter is not a second press. Pressing Enter again makes it.
#[cfg(target_os = "linux")]
#[test]
fn holding_enter_on_a_writes_button_writes_nothing() {
    let home = a_home_naming_one_project_settled("web-write-held-enter", WRITES_AS_CARTER);
    let needing_a_human = with_the_root_changed(THE_DESCRIBED_SUBTREE, |root| {
        root["labels"] = serde_json::json!(["human"]);
    });
    let (_web, address, tracker) = serving(&home, &needing_a_human, &["--writes"]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", THE_ROOT.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-dismiss')",
        GIVING_UP,
    );
    page.evaluate("document.querySelector('#write-dismiss>summary').focus()");
    page.hold("Enter");
    page.until(
        "document.querySelector('#write-dismiss').open \
         && document.activeElement === document.querySelector('#write-dismiss button')",
        GIVING_UP,
    );
    let dismissed = || {
        tracker
            .acted_calls()
            .iter()
            .any(|call| call.contains("human dismiss"))
    };
    std::thread::sleep(Duration::from_secs(1));
    assert!(!dismissed(), "{:?}", tracker.acted_calls());

    page.press("Enter", false);
    let giving_up = Instant::now() + GIVING_UP;
    while !dismissed() {
        assert!(
            Instant::now() < giving_up,
            "Enter on the button never dismissed: {:?}",
            tracker.acted_calls()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A bead that needs a human opens on its response, with its entry open.
/// Once the reader shuts it, it stays shut as the page is kept current, and
/// the first Escape on a page left as it opened closes the bead, as it does
/// on any other bead's page.
#[cfg(target_os = "linux")]
#[test]
fn a_page_opening_on_its_response_leaves_it_to_the_reader() {
    let home = a_home_naming_one_project_settled(
        "web-opens-on-the-response",
        &format!("[tui]\nrefresh_seconds = 1\n{WRITES_AS_CARTER}"),
    );
    let needing_a_human = with_the_root_changed(THE_DESCRIBED_SUBTREE, |root| {
        root["labels"] = serde_json::json!(["human"]);
    });
    let (_web, address, tracker) = serving(&home, &needing_a_human, &["--writes"]);
    let opened = format!("http://{address}/arkham/{}", THE_ROOT.0);
    let mut page = browser::Browser::open(&home, &opened, 1200, 800);
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-respond')",
        GIVING_UP,
    );
    assert_eq!(
        page.evaluate(
            "[...document.querySelectorAll('details[name=write][open]')].map((entry) => entry.id)"
        ),
        serde_json::json!(["write-respond"]),
        "the page opens on the response alone"
    );

    page.evaluate("document.querySelector('#write-respond>summary').click()");
    page.until("!document.querySelector('#write-respond').open", GIVING_UP);
    tracker.holds(&with_the_root_changed(&needing_a_human, |root| {
        root["priority"] = 0.into();
    }));
    page.until(
        "document.querySelector('#write-priority>summary').textContent === 'P0'",
        GIVING_UP,
    );
    assert_eq!(
        page.evaluate("document.querySelector('#write-respond').open"),
        serde_json::json!(false),
        "the redraw leaves the response shut"
    );

    page.evaluate("location.reload()");
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-respond')?.open",
        GIVING_UP,
    );
    page.evaluate("document.activeElement.blur()");
    page.press("Escape", false);
    page.until(
        &format!("location.pathname !== '/arkham/{}'", THE_ROOT.0),
        GIVING_UP,
    );
}

/// Escape never takes the reader away from words they typed into a write.
/// With a response typed and the focus moved on to its button, Escape shuts
/// the response's entry, though the page opened on it, and goes back to its
/// button. Another Escape leaves the bead's page where it is, words and all.
#[cfg(target_os = "linux")]
#[test]
fn escape_never_takes_the_reader_away_from_words_typed() {
    let home = a_home_naming_one_project_settled("web-escape-keeps-words", WRITES_AS_CARTER);
    let needing_a_human = with_the_root_changed(THE_DESCRIBED_SUBTREE, |root| {
        root["labels"] = serde_json::json!(["human"]);
    });
    let (_web, address, _tracker) = serving(&home, &needing_a_human, &["--writes"]);
    let opened = format!("/arkham/{}", THE_ROOT.0);
    let mut page = browser::Browser::open(&home, &format!("http://{address}{opened}"), 1200, 800);
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-respond')?.open",
        GIVING_UP,
    );
    page.evaluate("document.querySelector('#write-respond textarea').focus()");
    page.type_text("carbide");
    page.press("Tab", false);
    page.until(
        "document.activeElement === document.querySelector('#write-respond .foot button')",
        GIVING_UP,
    );
    page.press("Escape", false);
    page.until(
        "!document.querySelector('#write-respond').open \
         && document.activeElement === document.querySelector('#write-respond>summary')",
        GIVING_UP,
    );
    page.press("Escape", false);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        page.evaluate(
            "[location.pathname, document.querySelector('#write-respond textarea').value]"
        ),
        serde_json::json!([opened, "carbide"]),
        "the bead's page stays, with the words typed"
    );
}

/// A write's entry holding words stays as the page is kept current, though
/// the bead has changed so the page no longer offers that write, and it
/// makes no write until the bead takes it again. An entry holding no words
/// goes.
#[cfg(target_os = "linux")]
#[test]
fn a_redraw_keeps_an_entry_holding_words() {
    let home = a_home_naming_one_project_settled(
        "web-redraw-keeps-words",
        &format!("[tui]\nrefresh_seconds = 1\n{WRITES_AS_CARTER}"),
    );
    let needing_a_human = with_the_root_changed(THE_DESCRIBED_SUBTREE, |root| {
        root["labels"] = serde_json::json!(["human"]);
    });
    let (_web, address, tracker) = serving(&home, &needing_a_human, &["--writes"]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", THE_ROOT.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-respond')?.open",
        GIVING_UP,
    );
    page.evaluate("document.querySelector('#write-respond textarea').focus()");
    page.type_text("carbide");

    tracker.holds(&with_the_root_changed(THE_DESCRIBED_SUBTREE, |root| {
        root["priority"] = 0.into();
    }));
    page.until(
        "document.querySelector('#write-priority>summary').textContent === 'P0'",
        GIVING_UP,
    );
    assert_eq!(
        page.evaluate(
            "[document.querySelector('#write-respond textarea')?.value, \
              !!document.querySelector('#write-dismiss'), \
              document.querySelector('#write-respond .foot button').disabled]"
        ),
        serde_json::json!(["carbide", false, true]),
        "the response keeps its words and makes no write, and the dismissal goes"
    );
    page.evaluate("document.querySelector('#write-respond textarea').focus()");
    page.press("Enter", true);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !tracker
            .acted_calls()
            .iter()
            .any(|call| call.contains("human respond")),
        "Ctrl+Enter sent a response the bead no longer takes: {:?}",
        tracker.acted_calls()
    );

    tracker.holds(&with_the_root_changed(&needing_a_human, |root| {
        root["priority"] = 2.into();
    }));
    page.until(
        "document.querySelector('#write-priority>summary').textContent === 'P2' \
         && !document.querySelector('#write-respond .foot button').disabled",
        GIVING_UP,
    );
}

/// A response a redraw brings, because the bead has come to need a human,
/// does not open over an entry the reader has open: the comment being typed
/// stays open with its words, and the response waits shut. The browser
/// shuts an open entry inserted beside another open one of the same name.
#[cfg(target_os = "linux")]
#[test]
fn a_response_a_redraw_brings_leaves_the_open_entry_open() {
    let home = a_home_naming_one_project_settled(
        "web-redraw-brings-a-response",
        &format!("[tui]\nrefresh_seconds = 1\n{WRITES_AS_CARTER}"),
    );
    let (_web, address, tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &["--writes"]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", THE_ROOT.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-comment')",
        GIVING_UP,
    );
    page.press("m", false);
    page.until(
        "document.activeElement === document.querySelector('#write-comment textarea')",
        GIVING_UP,
    );
    page.type_text("the gate is shut");

    tracker.holds(&with_the_root_changed(THE_DESCRIBED_SUBTREE, |root| {
        root["labels"] = serde_json::json!(["human"]);
    }));
    page.until("document.querySelector('#write-respond')", GIVING_UP);
    assert_eq!(
        page.evaluate(
            "[document.querySelector('#write-comment').open, \
              document.querySelector('#write-comment textarea').value, \
              document.querySelector('#write-respond').open]"
        ),
        serde_json::json!([true, "the gate is shut", false]),
    );
}

/// Words typed into a write survive a redraw that finds the bead gone:
/// the entry stays beside the notice that says so, making no write. Once the
/// bead is back, the entry holding the words is the one its page shows,
/// open, and it writes again, though the bead now needs a human and its page
/// would otherwise open on the response. Through both, the cursor stays
/// where the reader left it, so what they type next goes on the end of the
/// words.
#[cfg(target_os = "linux")]
#[test]
fn words_typed_survive_the_bead_going_and_coming_back() {
    let home = a_home_naming_one_project_settled(
        "web-bead-goes-and-comes-back",
        &format!("[tui]\nrefresh_seconds = 1\n{WRITES_AS_CARTER}"),
    );
    let (_web, address, tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &["--writes"]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", A_BEAD_BENEATH.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-comment')",
        GIVING_UP,
    );
    page.press("m", false);
    page.until(
        "document.activeElement === document.querySelector('#write-comment textarea')",
        GIVING_UP,
    );
    page.type_text("the gate is shut");
    let the_comment = "[...document.querySelectorAll('#write-comment')].map((entry) => \
         [entry.querySelector('textarea').value, entry.querySelector('.foot button').disabled, \
          !!entry.closest('.writes'), entry.open, \
          document.activeElement === entry.querySelector('textarea') \
            && document.activeElement.selectionStart === document.activeElement.value.length])";

    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("the fixture is JSON");
    rows.retain(|row| row["id"] != A_BEAD_BENEATH.0);
    tracker.holds(&serde_json::to_string(&rows).expect("rows serialise"));
    page.until("document.querySelector('aside>p.missing')", GIVING_UP);
    assert_eq!(
        page.evaluate(the_comment),
        serde_json::json!([["the gate is shut", true, false, true, true]]),
        "the words stay beside the notice, and make no write"
    );

    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("the fixture is JSON");
    for row in &mut rows {
        if row["id"] == A_BEAD_BENEATH.0 {
            row["labels"] = serde_json::json!(["human"]);
        }
    }
    tracker.holds(&serde_json::to_string(&rows).expect("rows serialise"));
    page.until(
        "document.querySelector('aside .page #write-respond')",
        GIVING_UP,
    );
    assert_eq!(
        page.evaluate(&format!(
            "[{the_comment}, document.querySelector('#write-respond').open]"
        )),
        serde_json::json!([[["the gate is shut", false, true, true, true]], false]),
        "the bead's page shows the entry holding the words, and it writes again"
    );
}

/// A response kept through the bead going stays kept while the bead is
/// back but needs no human, and is the one the page offers, with its words,
/// once the bead needs a human again.
#[cfg(target_os = "linux")]
#[test]
fn a_kept_response_returns_with_its_write() {
    let home = a_home_naming_one_project_settled(
        "web-kept-response-returns",
        &format!("[tui]\nrefresh_seconds = 1\n{WRITES_AS_CARTER}"),
    );
    let labelled = |labels: serde_json::Value| {
        let mut rows: Vec<serde_json::Value> =
            serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("the fixture is JSON");
        for row in &mut rows {
            if row["id"] == A_BEAD_BENEATH.0 {
                row["labels"] = labels.clone();
            }
        }
        rows
    };
    let holding = |rows: &[serde_json::Value]| serde_json::to_string(rows).expect("rows serialise");
    let needing_a_human = labelled(serde_json::json!(["human"]));
    let (_web, address, tracker) = serving(&home, &holding(&needing_a_human), &["--writes"]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", A_BEAD_BENEATH.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-respond')?.open",
        GIVING_UP,
    );
    page.evaluate("document.querySelector('#write-respond textarea').focus()");
    page.type_text("carbide");
    let the_response = "[...document.querySelectorAll('#write-respond')].map((entry) => \
         [entry.querySelector('textarea').value, entry.querySelector('.foot button').disabled, \
          !!entry.closest('.writes')])";

    let gone: Vec<serde_json::Value> = needing_a_human
        .iter()
        .filter(|row| row["id"] != A_BEAD_BENEATH.0)
        .cloned()
        .collect();
    tracker.holds(&holding(&gone));
    page.until("document.querySelector('aside>p.missing')", GIVING_UP);
    tracker.holds(&holding(&labelled(serde_json::json!([]))));
    page.until("document.querySelector('aside .page')", GIVING_UP);
    assert_eq!(
        page.evaluate(the_response),
        serde_json::json!([["carbide", true, false]]),
        "the response stays kept while the bead needs no human"
    );

    tracker.holds(&holding(&needing_a_human));
    page.until(
        "document.querySelector('.writes #write-respond')",
        GIVING_UP,
    );
    assert_eq!(
        page.evaluate(the_response),
        serde_json::json!([["carbide", false, true]]),
        "the kept response is the one the page offers"
    );
}

/// Ctrl+Enter or Escape pressed while an input method is still composing
/// what it types is the input method's: it makes no write, and leaves the
/// entry open.
#[cfg(target_os = "linux")]
#[test]
fn a_key_pressed_while_composing_is_the_input_methods() {
    let home = a_home_naming_one_project_settled("web-write-composing", WRITES_AS_CARTER);
    let tracker = a_tracker_taking_the_comment(&home);
    let (_web, address) = bdi_web(&home, &tracker, &["--writes"], Some(&home), &[]);
    until_it_has_read(&address);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", THE_ROOT.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-comment')",
        GIVING_UP,
    );
    page.evaluate(
        "const entry = document.querySelector('#write-comment'); entry.open = true; \
         const field = entry.querySelector('textarea'); field.value = 'the gate is shut'; \
         field.focus(); field.dispatchEvent(new KeyboardEvent('keydown', \
           { key: 'Enter', ctrlKey: true, isComposing: true, bubbles: true, cancelable: true }))",
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        (
            tracker
                .acted_calls()
                .contains(&format!("carter {COMMENTED}")),
            page.evaluate("document.querySelector('#write-comment textarea')?.value")
        ),
        (false, serde_json::json!("the gate is shut")),
    );

    page.evaluate(
        "const empty = document.querySelector('#write-comment textarea'); empty.value = ''; \
         empty.dispatchEvent(new KeyboardEvent('keydown', \
           { key: 'Escape', isComposing: true, bubbles: true, cancelable: true }))",
    );
    assert_eq!(
        page.evaluate(
            "[document.querySelector('#write-comment').open, \
              document.activeElement === document.querySelector('#write-comment textarea')]"
        ),
        serde_json::json!([true, true]),
    );
}

/// Escape shuts the open entry the focus is in, though the menu of lists is
/// open too, and goes back to the entry's button.
#[cfg(target_os = "linux")]
#[test]
fn escape_shuts_the_entry_the_focus_is_in_first() {
    let home = a_home_naming_one_project_settled("web-escape-the-focused-entry", WRITES_AS_CARTER);
    let (_web, address, _tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &["--writes"]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", THE_ROOT.0),
        1200,
        800,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-priority')",
        GIVING_UP,
    );
    page.press("p", false);
    page.until(
        "document.activeElement?.closest('#write-priority form')",
        GIVING_UP,
    );
    page.evaluate("document.querySelector('details.lists').open = true");
    page.press("Escape", false);
    assert_eq!(
        page.evaluate(
            "[document.querySelector('#write-priority').open, \
              document.querySelector('details.lists').open, \
              document.activeElement === document.querySelector('#write-priority>summary')]"
        ),
        serde_json::json!([false, true, true]),
    );
}

/// The priority's choices fit the narrowest phone, so opening them moves
/// nothing off the screen.
#[cfg(target_os = "linux")]
#[test]
fn the_priority_choices_fit_the_narrowest_phone() {
    let home = a_home_naming_one_project_settled("web-priority-on-a-phone", WRITES_AS_CARTER);
    let (_web, address, _tracker) = serving(&home, THE_DESCRIBED_SUBTREE, &["--writes"]);
    let mut page = browser::Browser::open(
        &home,
        &format!("http://{address}/arkham/{}", THE_ROOT.0),
        280,
        640,
    );
    page.until(
        "document.readyState === 'complete' && document.querySelector('#write-priority')",
        GIVING_UP,
    );
    page.evaluate("document.querySelector('#write-priority').open = true");
    assert_eq!(
        page.evaluate(
            "[document.querySelector('#write-priority form').getBoundingClientRect().right <= innerWidth, \
              document.documentElement.scrollWidth <= innerWidth]"
        ),
        serde_json::json!([true, true]),
    );
}

/// `capture`, with dun-0tp as `change` leaves it.
fn with_the_root_changed(capture: &str, change: impl Fn(&mut serde_json::Value)) -> String {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(capture).expect("a capture of bd list --json");
    for row in &mut rows {
        if row["id"] == THE_ROOT.0 {
            change(row);
        }
    }
    serde_json::to_string(&rows).expect("rows serialise")
}

/// Open `path` through `relay`, and see that the browser fetched the page
/// and its script then asked only for the changes since it. Gives the
/// browser and the size of the page it fetched.
#[cfg(target_os = "linux")]
fn opened_once(home: &Path, relay: &Relay, path: &str) -> (browser::Browser, usize) {
    let url = format!("http://{}{path}", relay.address);
    let mut page = browser::Browser::open(home, &url, 1200, 600);
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
    (page, *page_bytes)
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

/// A comment, as the shim records the call past the `-C` naming the tracker.
const COMMENTED: &str = "comments add -- dun-0tp the gate is shut";

/// The config's table naming the one actor a writable `bdi web` writes as.
const WRITES_AS_CARTER: &str = "[web]\nactor = \"carter\"\n";

/// A database the shell that started `bdi web` exports never reaches bd, so
/// a comment lands where the project's own config says. Only the project's
/// tracker has an answer, so a redirected write is refused.
#[test]
fn a_database_the_caller_exports_does_not_redirect_a_write() {
    let home =
        a_home_naming_one_project_settled("web-write-ignores-exported-database", WRITES_AS_CARTER);
    let tracker = a_tracker_taking_the_comment(&home);
    let (_web, address) = bdi_web(
        &home,
        &tracker,
        &["--writes"],
        Some(&home),
        &[("BEADS_DOLT_SERVER_DATABASE", "kadath")],
    );
    until_it_has_read(&address);

    assert_eq!(the_comment_posted_to(&address), "HTTP/1.1 303 See Other");
    assert_eq!(tracker.calls_for("kadath"), Vec::<String>::new());
}

/// bd records a write under the actor it is told ahead of the one its
/// environment names, so the comment lands as the config's actor and not as
/// the one the shell that started `bdi web` exports.
#[test]
fn a_write_lands_under_the_actor_the_config_names() {
    let home =
        a_home_naming_one_project_settled("web-write-as-the-configured-actor", WRITES_AS_CARTER);
    let tracker = a_tracker_taking_the_comment(&home);
    let (_web, address) = bdi_web(
        &home,
        &tracker,
        &["--writes"],
        Some(&home),
        &[("BEADS_ACTOR", "pickman"), ("BD_ACTOR", "pickman")],
    );
    until_it_has_read(&address);

    assert_eq!(the_comment_posted_to(&address), "HTTP/1.1 303 See Other");
    let commented: Vec<_> = tracker
        .acted_calls()
        .into_iter()
        .filter(|call| call.ends_with(COMMENTED))
        .collect();
    assert_eq!(commented, [format!("carter {COMMENTED}")]);
}

/// A write lands in the tracker, and the page then shows the bead as it
/// now stands, read again for the write though the project does not poll
/// and no producer speaks for it.
#[test]
fn after_a_write_the_beads_page_shows_it_as_it_now_stands() {
    let home = a_home_naming_one_project_settled(
        "web-write-then-shown",
        &format!("poll = false\n{WRITES_AS_CARTER}"),
    );
    let named = home
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a home has a name");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let reprioritised = "update --priority=0 -- dun-0tp";
    tracker.answers_for(named, reprioritised, "");
    let (_web, address) = bdi_web(&home, &tracker, &["--writes"], Some(&home), &[]);
    until_it_has_read(&address);
    let shown_at = |priority: &str| {
        get(&address, "/arkham/dun-0tp")
            .1
            .contains(&format!(">{priority}</summary>"))
    };
    assert!(shown_at("P1"), "the bead is drawn as first read");

    tracker.holds(&with_the_root_changed(THE_DESCRIBED_SUBTREE, |root| {
        root["priority"] = 0.into();
    }));
    assert!(shown_at("P1"), "nothing reads the tracker again on its own");
    assert_eq!(
        posted_from_the_beads_page(&address, "priority", "text=0"),
        "HTTP/1.1 303 See Other"
    );

    assert!(tracker
        .calls()
        .iter()
        .any(|call| call.ends_with(reprioritised)));
    let giving_up = Instant::now() + GIVING_UP;
    while !shown_at("P0") {
        assert!(
            Instant::now() < giving_up,
            "the page never showed the write"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A writable `bdi web` with no actor would write as whoever runs it, so it
/// does not start, and names the key it wants.
#[test]
fn a_web_taking_writes_with_no_actor_configured_refuses_to_start() {
    let home = a_home_naming_one_project("web-writes-with-no-actor");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let spawned_by = std::process::id();
    let mut command = Command::new(env!("CARGO_BIN_EXE_bdi"));
    command
        .args(["web", "--listen", "127.0.0.1:0", "--writes"])
        .current_dir(&home)
        .env("HOME", &home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("XDG_RUNTIME_DIR")
        .envs(tracker.environment())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe { command.pre_exec(move || die_with(spawned_by)) };
    let mut web = Web(command.spawn().expect("bdi web starts"));
    let giving_up = Instant::now() + GIVING_UP;
    let exited = loop {
        if let Some(exited) = web.0.try_wait().expect("bdi web can be waited on") {
            break exited;
        }
        assert!(Instant::now() < giving_up, "bdi web started");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut answered = String::new();
    let mut said = String::new();
    web.0
        .stdout
        .take()
        .expect("its stdout")
        .read_to_string(&mut answered)
        .expect("its stdout is text");
    web.0
        .stderr
        .take()
        .expect("its stderr")
        .read_to_string(&mut said)
        .expect("its stderr is text");

    assert!(!exited.success(), "bdi web exited {exited}");
    assert_eq!(answered, "");
    assert!(said.contains("[web] actor"), "{said}");
    assert_eq!(tracker.calls(), Vec::<String>::new());
}

/// A tracker holding the described subtree that takes the comment
/// `the_comment_posted_to` makes.
fn a_tracker_taking_the_comment(home: &Path) -> ShimmedTracker {
    let named = home
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a home has a name");
    let tracker = ShimmedTracker::beside(home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    tracker.answers_for(named, COMMENTED, "");
    tracker
}

/// The status line the `bdi web` at `address` answers with when the bead's
/// page posts `COMMENTED`, as a browser posts the bead's form.
fn the_comment_posted_to(address: &str) -> String {
    posted_from_the_beads_page(address, "comment", "text=the+gate+is+shut")
}

/// The status line the `bdi web` at `address` answers with when the form on
/// dun-0tp's page that makes `write` posts `fields`, as a browser posts it:
/// from the page bdi web drew it on, with the token that form carries.
fn posted_from_the_beads_page(address: &str, write: &str, fields: &str) -> String {
    let (_, page) = get(address, "/arkham/dun-0tp");
    let action = format!("action=\"/arkham/dun-0tp/{write}");
    let token = page
        .split_once(action.as_str())
        .and_then(|(_, form)| form.split_once("name=\"token\" value=\""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(token, _)| token)
        .unwrap_or_else(|| panic!("the bead's page carries a token in its {write} form"));
    let mut stream = TcpStream::connect(address).expect("bdi web takes the connection");
    stream
        .set_read_timeout(Some(GIVING_UP))
        .expect("a read timeout");
    let form = format!("token={token}&{fields}");
    write!(
        stream,
        "POST /arkham/dun-0tp/{write} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{form}",
        form.len()
    )
    .expect("the request is sent");
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .expect("bdi web answers and closes");
    answer.lines().next().unwrap_or_default().to_string()
}

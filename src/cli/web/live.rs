//! An open page kept current: the one connection a page's script holds open,
//! and down it, as Server-Sent Events, the page its address answers each
//! time what the server holds changes.
//!
//! The first event on a connection is the whole page. Each one after it is
//! a patch against the page sent before it, unless the page itself is the
//! smaller, because a change to the forest is a few of its thousands of
//! lines.

use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::collect::http::{self, close, Head, LINGER};
use crate::view::address::Asked;

use super::{Answering, DrawnFrom, PATIENCE, TEXT, UNAVAILABLE};

/// The most pages kept current at once. These are apart from the requests
/// being answered, because each holds its connection for as long as its
/// page is open.
const MOST_KEPT_CURRENT: usize = 32;

/// How long a connection goes without an event before a comment is sent
/// down it, so that a proxy in front does not take it for idle, and a
/// reader who has gone is found.
const QUIET: Duration = Duration::from_secs(15);

/// The query key the page's script names the change it was drawn from by.
const SINCE: &str = "since";

/// The most lines a patch inserts or removes before the page is sent whole
/// instead. It bounds the work of finding the patch as well as its size.
const MOST_EDITS: usize = 1024;

/// The pages being kept current.
#[derive(Default)]
pub(super) struct Streams {
    open: AtomicUsize,
    /// Held while a page is drawn and patched for one of them, so that a
    /// change wakes them to draw one at a time rather than all at once.
    // ponytail: each draws its own page, so readers of one address draw it
    // once each; share a drawing per address if many read the same page.
    drawing: Mutex<()>,
}

/// Whether `head` asks for the changes to its page rather than the page.
pub(super) fn asks_for_changes(head: &Head) -> bool {
    head.method == "GET"
        && head
            .header("Accept")
            .is_some_and(|accept| accept.contains("text/event-stream"))
}

/// Keep the page `asked` names current down `stream`, on a thread of its
/// own, or refuse where as many pages as may be are kept current already.
pub(super) fn keep_current(
    answering: &Arc<Answering>,
    stream: TcpStream,
    head: &Head,
    asked: Asked,
) {
    let open = &answering.live.open;
    if open.fetch_add(1, Ordering::SeqCst) >= MOST_KEPT_CURRENT {
        open.fetch_sub(1, Ordering::SeqCst);
        http::reply(&stream, UNAVAILABLE, TEXT, b"busy\n", PATIENCE);
        close(stream, Instant::now() + LINGER);
        return;
    }
    let kept = Kept(Arc::clone(answering));
    let since = since(head);
    thread::spawn(move || send_changes(&kept.0, &stream, &asked, since));
}

/// One page being kept current, counted until it is dropped, panic or not.
struct Kept(Arc<Answering>);

impl Drop for Kept {
    fn drop(&mut self) {
        self.0.live.open.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The change a reconnecting script last heard of, or the one its page was
/// drawn from, which the script puts last in the query.
fn since(head: &Head) -> Option<u64> {
    head.header("Last-Event-ID")
        .or_else(|| {
            let (_, query) = head.target.split_once('?')?;
            query
                .rsplit('&')
                .find_map(|pair| pair.strip_prefix(SINCE)?.strip_prefix('='))
        })
        .and_then(|change| change.parse().ok())
}

/// Send `asked`'s page down `stream` each time it would be drawn otherwise,
/// until the reader goes.
fn send_changes(answering: &Answering, stream: &TcpStream, asked: &Asked, since: Option<u64>) {
    const HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
        Cache-Control: no-store\r\nX-Accel-Buffering: no\r\n\r\n";
    if http::send(stream, HEAD.as_bytes(), PATIENCE).is_err() {
        return;
    }
    let mut seen: Option<DrawnFrom> = since.map(|change| (change, false));
    let mut sent: Option<Vec<String>> = None;
    loop {
        let Some((held, from)) = answering.latest.after(seen, Instant::now() + QUIET) else {
            if http::send(stream, b":\n\n", PATIENCE).is_err() {
                return;
            }
            continue;
        };
        seen = Some(from);
        let (page, event) = {
            let _drawing = answering.live.drawing.lock();
            let page = lines(&super::drawn(asked, &held, &answering.cfg).1);
            let event = event(sent.as_deref(), &page, held.change);
            (page, event)
        };
        let Some(event) = event else {
            continue;
        };
        if http::send(stream, event.as_bytes(), PATIENCE).is_err() {
            return;
        }
        sent = Some(page);
    }
}

/// `page` as the lines an event carries. A carriage return would end a line
/// of an event, so it is written as the character reference it stands for.
fn lines(page: &str) -> Vec<String> {
    page.replace('\r', "&#13;")
        .split('\n')
        .map(str::to_string)
        .collect()
}

/// The event that takes a reader holding `sent` to `page`, numbered
/// `change`, or nothing where they are the same.
fn event(sent: Option<&[String]>, page: &[String], change: u64) -> Option<String> {
    let whole = || framed("page", change, page.iter().map(String::as_str));
    let Some(sent) = sent else {
        return Some(whole());
    };
    if sent == page {
        return None;
    }
    let patch = patch(sent, page)
        .map(|hunks| framed("patch", change, hunks.iter().map(String::as_str)))
        .filter(|patch| patch.len() < page.iter().map(String::len).sum());
    Some(patch.unwrap_or_else(whole))
}

/// An event of `kind`, numbered `change`, whose data is `lines`.
fn framed<'a>(kind: &str, change: u64, lines: impl Iterator<Item = &'a str>) -> String {
    let mut event = format!("event: {kind}\nid: {change}\n");
    for line in lines {
        event.push_str("data: ");
        event.push_str(line);
        event.push('\n');
    }
    event.push('\n');
    event
}

/// The patch that turns `before` into `after`, as the lines of a patch
/// event: each hunk is a line saying where in `before` it starts, how many
/// lines go and how many come, then the lines that come. The hunks run from
/// the last to the first, so each applies to the lines as the one before it
/// left them. Nothing where they differ by more than [`MOST_EDITS`] lines.
fn patch(before: &[String], after: &[String]) -> Option<Vec<String>> {
    let mut said = Vec::new();
    for hunk in hunks(before, after)?.iter().rev() {
        said.push(format!("{} {} {}", hunk.at, hunk.going, hunk.coming.len()));
        said.extend(after[hunk.coming.clone()].iter().cloned());
    }
    Some(said)
}

/// A run of lines of `before` that gives way to a run of lines of `after`.
#[derive(Debug, PartialEq, Eq)]
struct Hunk {
    at: usize,
    going: usize,
    coming: std::ops::Range<usize>,
}

/// The fewest runs of lines that turn `before` into `after`, first to last,
/// found by Myers's difference algorithm. Nothing where that takes more
/// than [`MOST_EDITS`] lines inserted or removed.
fn hunks(before: &[String], after: &[String]) -> Option<Vec<Hunk>> {
    let (n, m) = (before.len(), after.len());
    let most = MOST_EDITS.min(n + m);
    // Diagonal k = x - y is held at index k + most.
    let mut furthest = vec![0usize; 2 * most + 2];
    let mut trace = Vec::new();
    for d in 0..=most {
        trace.push(furthest.clone());
        for k in (0..=d).map(|step| 2 * step + most - d) {
            let down = k == most - d || (k != most + d && furthest[k - 1] < furthest[k + 1]);
            let mut x = if down {
                furthest[k + 1]
            } else {
                furthest[k - 1] + 1
            };
            let mut y = (x + most).checked_sub(k)?;
            while x < n && y < m && before[x] == after[y] {
                x += 1;
                y += 1;
            }
            furthest[k] = x;
            if x >= n && y >= m {
                return Some(walked_back(&trace, most, n, m));
            }
        }
    }
    None
}

/// The runs of lines the furthest points of each step in `trace` passed
/// over, from the end of both back to their starts, first to last.
fn walked_back(trace: &[Vec<usize>], most: usize, n: usize, m: usize) -> Vec<Hunk> {
    let (mut x, mut y) = (n, m);
    let mut hunks: Vec<Hunk> = Vec::new();
    for (d, furthest) in trace.iter().enumerate().skip(1).rev() {
        let k = x + most - y;
        let down = k == most - d || (k != most + d && furthest[k - 1] < furthest[k + 1]);
        let from_k = if down { k + 1 } else { k - 1 };
        let from_x = furthest[from_k];
        let from_y = from_x + most - from_k;
        let (at, going, coming) = if down {
            (from_x, 0, from_y..from_y + 1)
        } else {
            (from_x, 1, from_y..from_y)
        };
        match hunks.last_mut() {
            Some(next) if next.at == at + going && next.coming.start == coming.end => {
                next.at = at;
                next.going += going;
                next.coming.start = coming.start;
            }
            _ => hunks.push(Hunk { at, going, coming }),
        }
        (x, y) = (from_x, from_y);
    }
    hunks.reverse();
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the page's script makes of `sent` and a patch: the test's copy
    /// of it, applying each hunk in turn.
    fn applied(mut sent: Vec<String>, patch: &[String]) -> Vec<String> {
        let mut lines = patch.iter();
        while let Some(hunk) = lines.next() {
            let counts: Vec<usize> = hunk
                .split(' ')
                .map(|n| n.parse().expect("a count"))
                .collect();
            let came: Vec<String> = lines.by_ref().take(counts[2]).cloned().collect();
            sent.splice(counts[0]..counts[0] + counts[1], came);
        }
        sent
    }

    /// Every page of up to five lines drawn from two kinds of line, patched
    /// to every other: the patch applies, and inserts and removes no more
    /// lines than the fewest that could.
    #[test]
    fn every_patch_between_short_pages_applies_and_is_the_shortest() {
        let pages: Vec<Vec<String>> = (0..=5)
            .flat_map(|len| {
                (0..1usize << len).map(move |bits| {
                    (0..len)
                        .map(|at| if bits >> at & 1 == 1 { "b" } else { "a" }.to_string())
                        .collect()
                })
            })
            .collect();
        let common = |a: &[String], b: &[String]| {
            let mut longest = vec![vec![0; b.len() + 1]; a.len() + 1];
            for i in (0..a.len()).rev() {
                for j in (0..b.len()).rev() {
                    longest[i][j] = if a[i] == b[j] {
                        longest[i + 1][j + 1] + 1
                    } else {
                        longest[i + 1][j].max(longest[i][j + 1])
                    };
                }
            }
            longest[0][0]
        };
        for before in &pages {
            for after in &pages {
                let hunks = hunks(before, after).expect("hunks");
                let edits: usize = hunks
                    .iter()
                    .map(|hunk| hunk.going + hunk.coming.len())
                    .sum();
                assert_eq!(
                    edits,
                    before.len() + after.len() - 2 * common(before, after),
                    "{before:?} to {after:?}"
                );
                let patch = patch(before, after).expect("a patch");
                assert_eq!(
                    applied(before.clone(), &patch),
                    *after,
                    "{before:?} to {after:?}"
                );
            }
        }
    }

    #[test]
    fn a_patch_names_only_the_lines_that_changed() {
        let before: Vec<String> = (0..1000).map(|n| format!("line {n}")).collect();
        let mut after = before.clone();
        after[10] = "changed".to_string();
        after.insert(500, "came".to_string());
        after.remove(900);
        assert_eq!(
            hunks(&before, &after),
            Some(vec![
                Hunk {
                    at: 10,
                    going: 1,
                    coming: 10..11
                },
                Hunk {
                    at: 500,
                    going: 0,
                    coming: 500..501
                },
                Hunk {
                    at: 899,
                    going: 1,
                    coming: 900..900
                },
            ])
        );
    }

    #[test]
    fn pages_too_far_apart_for_a_patch_are_sent_whole() {
        let before: Vec<String> = (0..MOST_EDITS).map(|n| format!("before {n}")).collect();
        let after: Vec<String> = (0..MOST_EDITS).map(|n| format!("after {n}")).collect();
        assert_eq!(hunks(&before, &after), None);
        let event = event(Some(&before), &after, 7).expect("an event");
        assert!(event.starts_with("event: page\nid: 7\n"), "{event}");
    }

    #[test]
    fn the_first_event_is_the_page_and_an_unchanged_page_sends_nothing() {
        let drawn = lines("<!doctype html>\n<p>one</p>\r\n");
        assert_eq!(
            event(None, &drawn, 3),
            Some(
                "event: page\nid: 3\ndata: <!doctype html>\ndata: <p>one</p>&#13;\ndata: \n\n"
                    .to_string()
            )
        );
        assert_eq!(event(Some(&drawn), &drawn, 4), None);
    }

    #[test]
    fn a_page_says_the_change_it_was_drawn_from_and_a_reconnect_the_last_it_heard() {
        let asked = |sent: &str| {
            crate::collect::http::head(&mut sent.as_bytes(), &mut Vec::new()).expect("a head")
        };
        let head =
            asked("GET /a/b?open=p%3Aa&since=41 HTTP/1.1\r\nAccept: text/event-stream\r\n\r\n");
        assert!(asks_for_changes(&head));
        assert_eq!(since(&head), Some(41));
        let head = asked("GET /?since=41 HTTP/1.1\r\nLast-Event-ID: 44\r\n\r\n");
        assert!(!asks_for_changes(&head));
        assert_eq!(since(&head), Some(44));
        assert_eq!(since(&asked("GET / HTTP/1.1\r\n\r\n")), None);
        let head = asked("GET /?since=7&since=41 HTTP/1.1\r\n\r\n");
        assert_eq!(since(&head), Some(41), "the script's own comes last");
    }

    /// A page asking to be kept current while as many are kept current as
    /// may be is refused, and the refusal frees its worker soon even where
    /// the sender leaves the connection open.
    #[test]
    fn a_page_past_the_most_kept_current_is_refused_at_once() {
        use std::io::Read;
        use std::net::TcpListener;

        use super::super::{Config, Hosts, Latest};

        let answering = Arc::new(Answering {
            latest: Arc::new(Latest::new()),
            cfg: Config::naming(Vec::new()),
            hosts: Hosts::default(),
            live: Streams {
                open: AtomicUsize::new(MOST_KEPT_CURRENT),
                drawing: Mutex::default(),
            },
        });
        let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
        let mut sender =
            TcpStream::connect(listener.local_addr().expect("an address")).expect("a connection");
        let (stream, _) = listener.accept().expect("the connection");
        let sent = "GET / HTTP/1.1\r\nAccept: text/event-stream\r\n\r\n";
        let head = http::head(&mut sent.as_bytes(), &mut Vec::new()).expect("a head");
        let started = Instant::now();
        keep_current(
            &answering,
            stream,
            &head,
            Asked::of("/").expect("an address"),
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the refusal took {:?}",
            started.elapsed()
        );
        let mut answer = String::new();
        sender.read_to_string(&mut answer).expect("the answer");
        assert!(answer.starts_with("HTTP/1.1 503 "), "{answer}");
        assert_eq!(
            answering.live.open.load(Ordering::SeqCst),
            MOST_KEPT_CURRENT
        );
    }
}

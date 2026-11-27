//! A headless Chromium a test drives, for a test whose subject is what the
//! page's script does in a browser.
//!
//! It is driven over its debugging pipe rather than a port: messages of the
//! DevTools protocol, each a JSON object ended by a NUL, written to the
//! browser's descriptor 3 and read from its descriptor 4. So nothing but the
//! test can reach it, and it needs no client beyond `serde_json`.
//!
//! `chromium` is found on PATH, where the flake's test check and dev shell
//! put it on Linux.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::terminal::die_with;

/// How long the browser has to answer one message before the test fails
/// rather than hangs.
const ANSWERING: Duration = Duration::from_secs(60);

/// One page open in a headless Chromium, killed and reaped when it is
/// dropped.
pub struct Browser {
    chromium: Child,
    to: File,
    from: File,
    /// What the browser has said that is not yet read as a whole message.
    unread: Vec<u8>,
    asked: u64,
    page: String,
    said: std::path::PathBuf,
}

impl Browser {
    /// A browser keeping its profile under `home`, with one page `width` by
    /// `height` pixels, open at `url`.
    pub fn open(home: &Path, url: &str, width: u32, height: u32) -> Self {
        let (theirs_to_read, to) = pipe();
        let (from, theirs_to_write) = pipe();
        let (read, write) = (theirs_to_read.as_raw_fd(), theirs_to_write.as_raw_fd());
        let spawned_by = std::process::id();
        let said = home.join("chromium.log");
        let mut command = Command::new("chromium");
        command
            .args([
                "--headless",
                "--remote-debugging-pipe",
                "--no-sandbox",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
            ])
            .arg(format!(
                "--user-data-dir={}",
                home.join("chromium").display()
            ))
            .env("HOME", home)
            // Its singleton socket goes under TMPDIR, which the flake makes too
            // long for a socket's path; the home is short enough.
            .env("TMPDIR", home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(&said).expect("the browser's log is ours to write"));
        unsafe {
            command.pre_exec(move || {
                die_with(spawned_by)?;
                for (fd, onto) in [(read, 3), (write, 4)] {
                    if libc::dup2(fd, onto) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            })
        };
        let chromium = command
            .spawn()
            .expect("chromium starts; the flake's dev shell has it on Linux");
        drop((theirs_to_read, theirs_to_write));
        let mut browser = Self {
            chromium,
            to: File::from(to),
            from: File::from(from),
            unread: Vec::new(),
            asked: 0,
            page: String::new(),
            said,
        };
        let target = browser.send("Target.createTarget", json!({ "url": "about:blank" }));
        let attached = browser.send(
            "Target.attachToTarget",
            json!({ "targetId": target["targetId"], "flatten": true }),
        );
        browser.page = attached["sessionId"]
            .as_str()
            .expect("the page has a session")
            .to_string();
        browser.on_page(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }),
        );
        browser.on_page(
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        );
        browser.on_page("Page.navigate", json!({ "url": url }));
        browser
    }

    /// What `expression` comes to on the page, awaited where it is a
    /// promise. A throw is a failure of the test.
    pub fn evaluate(&mut self, expression: &str) -> Value {
        let answer = self.on_page(
            "Runtime.evaluate",
            json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
        );
        assert!(
            answer.get("exceptionDetails").is_none(),
            "{expression} threw: {answer}"
        );
        answer["result"]["value"].clone()
    }

    /// Press `key` as a keyboard does, down then up, to whatever has the
    /// focus, holding Ctrl where `with_ctrl`. `key` is a letter, which a
    /// capital names with Shift held, or `Enter`, `Escape` or `Tab`.
    pub fn press(&mut self, key: &str, with_ctrl: bool) {
        self.stroke(key, with_ctrl, 0);
    }

    /// Hold `key` down long enough for the keyboard to repeat it, then let
    /// it go.
    pub fn hold(&mut self, key: &str) {
        self.stroke(key, false, 3);
    }

    /// `key` down, `repeats` times more as the keyboard repeats it, then up.
    fn stroke(&mut self, key: &str, with_ctrl: bool, repeats: usize) {
        let (code, keycode, text) = match key {
            "Enter" => ("Enter".to_string(), 13, "\r".to_string()),
            "Escape" => ("Escape".to_string(), 27, String::new()),
            "Tab" => ("Tab".to_string(), 9, "\t".to_string()),
            letter => {
                let upper = letter.to_ascii_uppercase();
                let keycode = upper.chars().next().expect("a letter") as u32;
                (format!("Key{upper}"), keycode, letter.to_string())
            }
        };
        let shift = key.len() == 1 && key.chars().all(|letter| letter.is_ascii_uppercase());
        let modifiers = if with_ctrl { 2 } else { 0 } | if shift { 8 } else { 0 };
        let text = if with_ctrl { String::new() } else { text };
        let down = (0..=repeats).map(|repeated| ("keyDown", repeated > 0));
        for (kind, repeated) in down.chain([("keyUp", false)]) {
            let mut event = json!({
                "type": kind,
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": keycode,
                "modifiers": modifiers,
                "autoRepeat": repeated,
            });
            if kind == "keyDown" && !text.is_empty() {
                event["text"] = json!(text);
            }
            self.on_page("Input.dispatchKeyEvent", event);
        }
    }

    /// Type `text` into whatever has the focus, as a keyboard would.
    pub fn type_text(&mut self, text: &str) {
        self.on_page("Input.insertText", json!({ "text": text }));
    }

    /// Wait until `expression` comes to true on the page, failing once
    /// `giving_up` passes.
    pub fn until(&mut self, expression: &str, giving_up: Duration) {
        let giving_up = Instant::now() + giving_up;
        while self.evaluate(&format!("!!({expression})")) != Value::Bool(true) {
            assert!(
                Instant::now() < giving_up,
                "the page never came to {expression}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn on_page(&mut self, method: &str, params: Value) -> Value {
        let page = self.page.clone();
        self.sent(json!({ "method": method, "params": params, "sessionId": page }))
    }

    fn send(&mut self, method: &str, params: Value) -> Value {
        self.sent(json!({ "method": method, "params": params }))
    }

    /// Send `message` numbered, and read what comes back until the answer
    /// bearing its number, passing over the events between. The answer has
    /// `ANSWERING` to come.
    fn sent(&mut self, mut message: Value) -> Value {
        self.asked += 1;
        message["id"] = json!(self.asked);
        let mut sending = serde_json::to_vec(&message).expect("a message serialises");
        sending.push(0);
        self.to
            .write_all(&sending)
            .expect("the browser takes a message");
        let giving_up = Instant::now() + ANSWERING;
        loop {
            while let Some(end) = self.unread.iter().position(|&byte| byte == 0) {
                let heard: Vec<u8> = self.unread.drain(..=end).collect();
                let heard: Value =
                    serde_json::from_slice(&heard[..end]).expect("the browser says JSON");
                if heard["id"] == message["id"] {
                    assert!(
                        heard.get("error").is_none(),
                        "{message} was refused: {heard}"
                    );
                    return heard["result"].clone();
                }
            }
            if !self.answers_within(giving_up.saturating_duration_since(Instant::now())) {
                panic!(
                    "the browser gave no answer to {message} within {ANSWERING:?}, saying:\n{}",
                    self.said()
                );
            }
            let mut said = [0; 1 << 16];
            let got = self.from.read(&mut said).expect("the browser answers");
            if got == 0 {
                panic!(
                    "the browser went before answering {message}, saying:\n{}",
                    self.said()
                );
            }
            self.unread.extend_from_slice(&said[..got]);
        }
    }
}

impl Browser {
    /// Whether the browser has something to say before `within` passes.
    fn answers_within(&self, within: Duration) -> bool {
        let mut answer = libc::pollfd {
            fd: self.from.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let waiting = i32::try_from(within.as_millis()).expect("a wait poll can take");
        unsafe { libc::poll(&mut answer, 1, waiting) > 0 }
    }

    fn said(&self) -> String {
        std::fs::read_to_string(&self.said).unwrap_or_default()
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.chromium.kill();
        let _ = self.chromium.wait();
    }
}

/// A pipe, as its reading end then its writing end, each closed on exec
/// unless it is copied onto a descriptor of the child's. Both are above 4,
/// so copying one onto 3 or 4 cannot close the other first.
fn pipe() -> (OwnedFd, OwnedFd) {
    let mut ends = [0; 2];
    assert_eq!(
        unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) },
        0,
        "a pipe: {}",
        std::io::Error::last_os_error()
    );
    let [read, write] = ends.map(|end| unsafe { OwnedFd::from_raw_fd(end) });
    (above_four(&read), above_four(&write))
}

fn above_four(fd: &OwnedFd) -> OwnedFd {
    let copy = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 5) };
    assert!(copy >= 5, "a copy: {}", std::io::Error::last_os_error());
    unsafe { OwnedFd::from_raw_fd(copy) }
}

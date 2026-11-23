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
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::terminal::die_with;

/// One page open in a headless Chromium, killed and reaped when it is
/// dropped.
pub struct Browser {
    chromium: Child,
    to: File,
    from: BufReader<File>,
    asked: u64,
    page: String,
}

impl Browser {
    /// A browser keeping its profile under `home`, with one page `width` by
    /// `height` pixels, open at `url`.
    pub fn open(home: &Path, url: &str, width: u32, height: u32) -> Self {
        let (theirs_to_read, to) = pipe();
        let (from, theirs_to_write) = pipe();
        let (read, write) = (theirs_to_read.as_raw_fd(), theirs_to_write.as_raw_fd());
        let spawned_by = std::process::id();
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
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
            from: BufReader::new(File::from(from)),
            asked: 0,
            page: String::new(),
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
    /// bearing its number, passing over the events between.
    fn sent(&mut self, mut message: Value) -> Value {
        self.asked += 1;
        message["id"] = json!(self.asked);
        let mut said = serde_json::to_vec(&message).expect("a message serialises");
        said.push(0);
        self.to
            .write_all(&said)
            .expect("the browser takes a message");
        loop {
            let mut heard = Vec::new();
            self.from
                .read_until(0, &mut heard)
                .expect("the browser answers");
            assert_eq!(
                heard.pop(),
                Some(0),
                "the browser went before answering {message}"
            );
            let heard: Value = serde_json::from_slice(&heard).expect("the browser says JSON");
            if heard["id"] == message["id"] {
                assert!(
                    heard.get("error").is_none(),
                    "{message} was refused: {heard}"
                );
                return heard["result"].clone();
            }
        }
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

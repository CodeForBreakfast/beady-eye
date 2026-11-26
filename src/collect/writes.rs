//! The light-touch writes a person makes from the terminal, made in turn on a
//! thread of their own so that a tracker slow to answer never holds the screen.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use crate::collect::bd::{self, Write};
use crate::collect::run::{self, Env, RealRunner, RunFailure};
use crate::config::Project;
use crate::model::join::BeadKey;

/// Where a write is handed to be made.
pub trait Writes {
    /// Make `write` to `bead` in `project`'s tracker, given what the person
    /// typed or the holder a release names, and say how it went once bd has
    /// answered.
    fn write(&self, project: &Project, bead: BeadKey, write: Write, given: String);
}

/// How a write went: the bead it was made to, and why it was not made where
/// it was not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrote {
    pub bead: BeadKey,
    pub write: Write,
    pub refused: Option<String>,
}

/// Writes through `bdi bd`'s own path, one at a time in the order they were
/// made, with every answer sent on to the channel it was made with. Let go,
/// it makes every write still queued before it goes, so a reader who quits
/// straight after a write still has it made, within the deadline the
/// terminal gives a leaving `bdi`.
pub struct Writer {
    queue: Option<Sender<Queued>>,
    making: Option<JoinHandle<()>>,
}

struct Queued {
    project: Project,
    bead: BeadKey,
    write: Write,
    given: String,
}

impl Writer {
    /// A writer that tells the watcher at `watcher` of each write bd takes.
    pub fn new<A: From<Wrote> + Send + 'static>(
        answers: Sender<A>,
        watcher: Option<PathBuf>,
    ) -> Self {
        Self::making(answers, move |project: &Project, asked: &[String]| {
            written(project, asked, watcher.as_deref())
        })
    }

    fn making<A: From<Wrote> + Send + 'static>(
        answers: Sender<A>,
        make: impl Fn(&Project, &[String]) -> anyhow::Result<()> + Send + 'static,
    ) -> Self {
        let (queue, queued) = mpsc::channel::<Queued>();
        let making = thread::spawn(move || {
            for Queued {
                project,
                bead,
                write,
                given,
            } in queued
            {
                let refused = make(&project, &write.asked(&bead.id, &given))
                    .err()
                    .map(|why| format!("{why:#}"));
                let wrote = Wrote {
                    bead,
                    write,
                    refused,
                };
                let _ = answers.send(wrote.into());
            }
        });
        Self {
            queue: Some(queue),
            making: Some(making),
        }
    }
}

impl Writes for Writer {
    fn write(&self, project: &Project, bead: BeadKey, write: Write, given: String) {
        if let Some(queue) = &self.queue {
            let _ = queue.send(Queued {
                project: project.clone(),
                bead,
                write,
                given,
            });
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.queue.take();
        if let Some(making) = self.making.take() {
            let _ = making.join();
        }
    }
}

/// Make a write with the environment the terminal reads the project with,
/// the caller's choice of tracker included, so that it reaches the tracker
/// the bead on the screen was read from.
fn written(project: &Project, asked: &[String], watcher: Option<&Path>) -> anyhow::Result<()> {
    bd::make(
        &RealRunner,
        project,
        asked,
        None,
        watcher,
        |argv, env| made("bd", argv, Some(&project.path), env),
        |()| true,
    )
}

/// Run a write, and where it is refused say why in the words it was refused
/// in, on one line. Only the person who made the write reads them, on their
/// own terminal, where `bdi bd` shows bd's stderr too.
fn made(program: &str, argv: &[&str], cwd: Option<&Path>, env: &Env) -> anyhow::Result<()> {
    let out = run::captured(program, argv, cwd, env)?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if said.is_empty() {
        return Err(RunFailure::from_exit(program, out.status.code(), "").into());
    }
    anyhow::bail!("{}", said.strip_prefix("Error: ").unwrap_or(&said))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn arkham() -> Project {
        toml::from_str("name = \"arkham\"\npath = \"/srv/work/arkham\"").expect("a project")
    }

    fn bead() -> BeadKey {
        BeadKey {
            project: "arkham".to_string(),
            id: "dun-0tp".to_string(),
        }
    }

    #[test]
    fn writes_reach_bd_in_the_order_they_were_made() {
        let (answers, answered) = mpsc::channel::<Wrote>();
        let reached = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&reached);
        let writer = Writer::making(answers, move |_: &Project, asked: &[String]| {
            if asked.iter().any(|word| word == "--priority=1") {
                thread::sleep(Duration::from_millis(50));
            }
            seen.lock().expect("the writes").push(asked.to_vec());
            Ok(())
        });

        writer.write(&arkham(), bead(), Write::Priority, "1".to_string());
        writer.write(&arkham(), bead(), Write::Priority, "2".to_string());
        for _ in 0..2 {
            answered
                .recv_timeout(Duration::from_secs(5))
                .expect("an answer");
        }

        assert_eq!(
            *reached.lock().expect("the writes"),
            vec![
                Write::Priority.asked("dun-0tp", "1"),
                Write::Priority.asked("dun-0tp", "2"),
            ]
        );
    }

    /// A refused write says why in bd's own words, on one line, because the
    /// person who made it is the one reading them.
    #[test]
    fn a_refused_write_says_why_in_the_words_it_was_refused_in() {
        let refused = made(
            "sh",
            &[
                "-c",
                "printf 'Error: invalid --until format \"someday\"\\nHint: try tomorrow\\n' >&2; exit 1",
            ],
            None,
            &Env::new(),
        )
        .expect_err("refused");

        assert_eq!(
            format!("{refused:#}"),
            "invalid --until format \"someday\" Hint: try tomorrow"
        );
    }

    /// A refusal bd gave no words for is still said, in bdi's own.
    #[test]
    fn a_write_refused_in_silence_says_how_it_ended() {
        let refused = made("sh", &["-c", "exit 3"], None, &Env::new()).expect_err("refused");

        assert_eq!(
            format!("{refused:#}"),
            "sh exited 3 for a reason bdi cannot place"
        );
    }

    /// A reader who quits with writes still queued has them made: the
    /// writer is let go after the terminal is put back, and waits for them.
    #[test]
    fn a_writer_let_go_makes_every_write_still_queued() {
        let (answers, answered) = mpsc::channel::<Wrote>();
        drop(answered);
        let reached = Arc::new(Mutex::new(0));
        let seen = Arc::clone(&reached);
        let writer = Writer::making(answers, move |_: &Project, _: &[String]| {
            thread::sleep(Duration::from_millis(20));
            *seen.lock().expect("the writes") += 1;
            Ok(())
        });

        writer.write(&arkham(), bead(), Write::Reopen, String::new());
        writer.write(&arkham(), bead(), Write::Dismiss, String::new());
        drop(writer);

        assert_eq!(*reached.lock().expect("the writes"), 2);
    }
}

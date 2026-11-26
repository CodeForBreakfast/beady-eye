//! The light-touch writes a person makes from the terminal, made in turn on a
//! thread of their own so that a tracker slow to answer never holds the screen.

use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::collect::bd::{self, Write};
use crate::collect::run::{RealRunner, Runner};
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
/// made, with every answer sent on to the channel it was made with.
pub struct Writer {
    queue: Sender<Queued>,
}

struct Queued {
    project: Project,
    bead: BeadKey,
    write: Write,
    given: String,
}

impl Writer {
    pub fn new<A: From<Wrote> + Send + 'static>(answers: Sender<A>) -> Self {
        Self::making(answers, written)
    }

    fn making<A: From<Wrote> + Send + 'static>(
        answers: Sender<A>,
        make: impl Fn(&Project, &[String]) -> anyhow::Result<()> + Send + 'static,
    ) -> Self {
        let (queue, queued) = mpsc::channel::<Queued>();
        thread::spawn(move || {
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
                if answers.send(wrote.into()).is_err() {
                    return;
                }
            }
        });
        Self { queue }
    }
}

impl Writes for Writer {
    fn write(&self, project: &Project, bead: BeadKey, write: Write, given: String) {
        let _ = self.queue.send(Queued {
            project: project.clone(),
            bead,
            write,
            given,
        });
    }
}

/// Make a write with the environment the terminal reads the project with,
/// the caller's choice of tracker included, so that it reaches the tracker
/// the bead on the screen was read from.
fn written(project: &Project, asked: &[String]) -> anyhow::Result<()> {
    let (argv, env) = bd::writing(&RealRunner, project, asked)?;
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    RealRunner.run("bd", &argv, Some(&project.path), &env)?;
    Ok(())
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
}

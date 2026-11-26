//! The light-touch writes a person makes from the terminal, each made on a
//! thread of its own so that a tracker slow to answer never holds the screen.

use std::sync::mpsc::Sender;
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

/// Writes through `bdi bd`'s own path, with every answer sent on to the
/// channel it was made with.
pub struct Writer<A> {
    answers: Sender<A>,
}

impl<A> Writer<A> {
    pub fn new(answers: Sender<A>) -> Self {
        Self { answers }
    }
}

impl<A: From<Wrote> + Send + 'static> Writes for Writer<A> {
    fn write(&self, project: &Project, bead: BeadKey, write: Write, given: String) {
        let project = project.clone();
        let answers = self.answers.clone();
        thread::spawn(move || {
            let refused = written(&project, &write.asked(&bead.id, &given))
                .err()
                .map(|why| format!("{why:#}"));
            let _ = answers.send(
                Wrote {
                    bead,
                    write,
                    refused,
                }
                .into(),
            );
        });
    }
}

fn written(project: &Project, asked: &[String]) -> anyhow::Result<()> {
    let (argv, env) = bd::writing(&RealRunner, project, asked)?;
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    RealRunner.run("bd", &argv, Some(&project.path), &env)?;
    Ok(())
}

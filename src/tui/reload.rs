//! When the config the run is working to is read again.
//!
//! One more deadline in the set the loop already sleeps on, and the smallest
//! thing that can notice a config edit: the file is read when its deadline
//! comes round, and parsed only where its text is not the text the last check
//! saw. Nothing here watches a file. A watch is a thread, a dependency and a
//! failure of its own, on a file that changes a few times a week — and what
//! it would buy is noticing an edit inside a second rather than inside the
//! interval below.
//!
//! **The text is what is compared, and not the file's timestamp.** A stamp
//! answers *has this been written* only as well as whoever last wrote it
//! allowed: two writes inside one of the kernel's coarse ticks carry one
//! stamp, and a file put back by anything that preserves times — `cp -p`,
//! `rsync -a`, an unpacked archive — carries the stamp it was taken with.
//! Either leaves a config the reader has genuinely changed sitting under a
//! stamp `bdi` has already seen, and a check that skips on that skips for
//! ever. Reading a few kilobytes from the page cache every couple of seconds
//! costs less than the case it rules out, and it is the parse and the `git`
//! calls behind it that a check is worth skipping anyway.
//!
//! What a re-read may never do is lose the config the run is working to. A
//! file that will not parse and a file that will not open leave the config in
//! force exactly as it was: falling back to defaults would silently drop every
//! project the reader had configured, which is the disappearance `bdi` is
//! built not to do.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::config::Config;

use super::due::due_after;

/// How long after each check the next one falls due.
///
/// A constant rather than a config setting, because a setting would have to
/// be read out of the file it governs: a config edit that lengthened this
/// would only take effect on the interval it replaced.
pub(crate) const CHECKED_EVERY: Duration = Duration::from_secs(2);

/// What a check found.
///
/// Two questions rather than one, and they come apart: whether a config the
/// run had not read is now in force, and whether the last read of the file
/// succeeded. `Unchanged` is the state that separates them — a read that
/// worked and brought nothing new — and it is what a reader gets for undoing
/// a broken edit. Anything that answered only the first question would leave
/// that reader looking at a screen still saying their config is broken.
/// The config rides on `Fresh` rather than beside it, so that *the reader has
/// written something new* and *here is what they wrote* cannot come apart.
/// Everything the run works to is read out of the one value, and a key added
/// to the config file is read under the file in force because there is no
/// second place for it to be missed from. A borrow, so a verdict the foot
/// draws stays cheap to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reloaded<'a> {
    /// Nothing came of it: the check is not due, or the file says exactly
    /// what it said when the last check read it.
    Untouched,
    /// The file was read and says what the config in force already says.
    Unchanged,
    /// The reader has written a config this run had not read. It is the
    /// config in force from here on.
    Fresh(&'a Config),
    /// The file would not open, or would not parse. The config in force
    /// stands untouched and the reader is told.
    Broken,
}

/// The file's text as this run reads a config — scoped, rooted and told
/// where each project is worked, exactly as the read at startup was.
///
/// Handed to a `Reload` rather than done by one, because all of that is the
/// command line's, which is settled where `bdi` is run and not here.
pub(crate) type Parses = Box<dyn Fn(&str) -> anyhow::Result<Config>>;

/// The config file, the config the run is working to, and when the two are
/// next compared.
///
/// Armed from the check before it rather than from an answer, which is what
/// makes it unlike `Armed`: there is no round trip to wait on, so a check
/// that finds nothing arms the next one just as one that reloads does.
pub(crate) struct Reload {
    path: PathBuf,
    /// How long after each check the next falls due.
    every: Duration,
    /// When the next check falls due, or nothing where the interval is too
    /// long to reach — a reload the loop never wakes for, as `due_after`
    /// says.
    at: Option<DateTime<Utc>>,
    /// What the file said, as the last check that could open it read it.
    ///
    /// Nothing before the first check, because the config in force was read
    /// before this existed and nothing here saw the text it came from. And
    /// nothing again after a check that could not open the file, which is
    /// the same statement: there is no text this run has seen at that path.
    /// A check with nothing remembered parses whatever it finds.
    ///
    /// Text that would not parse is remembered like any other, and that is
    /// what leaves the notice up between one check and the next: the file
    /// still says what it said, so there is nothing new to say about it.
    said: Option<String>,
    /// The config this run is working to. Replaced only by a read that
    /// parses and says something new.
    in_force: Config,
    parses: Parses,
}

impl Reload {
    pub(crate) fn watching(
        path: PathBuf,
        every: Duration,
        in_force: Config,
        parses: Parses,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            path,
            every,
            at: due_after(now, every),
            said: None,
            in_force,
            parses,
        }
    }

    /// How long until the config is next compared against the file, or
    /// nothing where it never will be.
    pub(super) fn checks_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.at
            .map(|at| (at - now).to_std().unwrap_or(Duration::ZERO))
    }

    /// Compare the config in force against the file where the check is due.
    ///
    /// Called on every wake, whichever deadline woke the loop, the way the
    /// band's pane read is: a check that is not due answers `Untouched` and
    /// touches nothing.
    pub(super) fn checks(&mut self, now: DateTime<Utc>) -> Reloaded<'_> {
        if !self.at.is_some_and(|at| at <= now) {
            return Reloaded::Untouched;
        }
        self.at = due_after(now, self.every);
        self.reads()
    }

    /// Read the file, parse it where it says something the last check did
    /// not see, and say what came of it.
    ///
    /// An editor that saves by writing a temp file and renaming it over this
    /// one leaves a window in which the path resolves to nothing. On this
    /// interval that window is very unlikely to be caught, and where it is,
    /// the notice goes up and comes down again an interval later. That flap
    /// is accepted rather than unnoticed: a config that has genuinely gone
    /// away is the case worth telling the reader about, and nothing here can
    /// tell the two apart at the instant it looks.
    fn reads(&mut self) -> Reloaded<'_> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            self.said = None;
            return Reloaded::Broken;
        };
        if self.said.as_deref() == Some(text.as_str()) {
            return Reloaded::Untouched;
        }
        let parsed = (self.parses)(&text);
        self.said = Some(text);
        let Ok(written) = parsed else {
            return Reloaded::Broken;
        };
        if written == self.in_force {
            return Reloaded::Unchanged;
        }
        self.in_force = written;
        Reloaded::Fresh(&self.in_force)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs::{File, FileTimes};
    use std::rc::Rc;
    use std::time::SystemTime;

    const ORBITAL: &str = "[[projects]]\nname = \"orbital\"\npath = \"/srv/work/orbital\"\n";
    const ORBITAL_AND_FERRY: &str = "[[projects]]\nname = \"orbital\"\npath = \"/srv/work/orbital\"\n\n[[projects]]\nname = \"ferry\"\npath = \"/srv/work/ferry\"\n";
    const NOT_TOML: &str = "[[projects]\nthis is not toml\n";

    const EVERY_TWO_SECONDS: Duration = Duration::from_secs(2);

    /// The config the check found, or a failure naming what it found instead.
    ///
    /// Asserting on the config rather than on the verdict is what the verdict
    /// carrying it is for: a check that answers `Fresh` about the wrong
    /// config is the failure the rest of the run cannot see, since it is
    /// handed nothing else to read the file's meaning off.
    #[track_caller]
    fn came_into_force(reloaded: Reloaded<'_>) -> &Config {
        match reloaded {
            Reloaded::Fresh(written) => written,
            found => panic!("the check answered {found:?} rather than with a config"),
        }
    }

    /// The config `ORBITAL_AND_FERRY` says, which is what every check here
    /// that finds something new finds.
    fn both() -> Config {
        Config::from_toml(ORBITAL_AND_FERRY).expect("the fixture parses")
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).expect("an instant inside the epoch")
    }

    fn a_config(text: &str) -> Config {
        Config::from_toml(text).expect("the fixture parses")
    }

    /// A config file of this test's own, written with `text` and stamped
    /// `written_at` seconds into the epoch.
    fn a_config_file(named: &str, text: &str, written_at: u64) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("bdi-reload-{named}-{}.toml", std::process::id()));
        written(&path, text, written_at);
        path
    }

    /// Written and stamped.
    ///
    /// Every write here carries a stamp of the test's choosing, so a test
    /// that puts one config in place of another **under the stamp the file
    /// already had** can say so exactly. That is the case a check comparing
    /// timestamps skips for ever, and there is no waiting for it: the kernel
    /// grants it on its own inside one coarse tick, and `cp -p` and an
    /// unpacked archive grant it whenever they like.
    fn written(path: &PathBuf, text: &str, written_at: u64) {
        std::fs::write(path, text).expect("the config is ours to write");
        File::options()
            .write(true)
            .open(path)
            .expect("the config is ours to open")
            .set_times(
                FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(written_at)),
            )
            .expect("the config's mtime is ours to set");
    }

    /// A reload watching `path`, working to `in_force`, parsing as `bdi`
    /// does — and the count of how often it has parsed, which is how a test
    /// says the file was read rather than skipped.
    fn watching(path: PathBuf, in_force: &str) -> (Reload, Rc<Cell<usize>>) {
        let parsed = Rc::new(Cell::new(0));
        let counted = Rc::clone(&parsed);
        let reload = Reload::watching(
            path,
            EVERY_TWO_SECONDS,
            a_config(in_force),
            Box::new(move |text| {
                counted.set(counted.get() + 1);
                Config::from_toml(text)
            }),
            at(0),
        );
        (reload, parsed)
    }

    /// The first check is always a read: the config in force was read before
    /// the reload existed, and nothing in it saw the file that config came
    /// from.
    #[test]
    fn the_first_check_reads_the_file_the_config_in_force_came_from() {
        let path = a_config_file("first-check", ORBITAL, 100);
        let (mut reload, parsed) = watching(path, ORBITAL);

        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);
        assert_eq!(parsed.get(), 1);
    }

    /// A file saying what it already said is read again — that is how this
    /// knows — and not parsed again, which is where the cost is: a parse
    /// carries the scoping and a `git worktree list` per project behind it.
    #[test]
    fn a_config_saying_what_it_already_said_is_not_parsed_again() {
        let path = a_config_file("untouched", ORBITAL, 100);
        let (mut reload, parsed) = watching(path, ORBITAL);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        assert_eq!(reload.checks(at(4)), Reloaded::Untouched);
        assert_eq!(
            parsed.get(),
            1,
            "the file was parsed once, at the first check"
        );
    }

    #[test]
    fn a_config_the_reader_has_written_comes_into_force() {
        let path = a_config_file("written", ORBITAL, 100);
        let (mut reload, _) = watching(path.clone(), ORBITAL);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        written(&path, ORBITAL_AND_FERRY, 200);

        assert_eq!(came_into_force(reload.checks(at(4))), &both());
    }

    /// A config whose text changed and whose meaning did not — a comment
    /// added, a table moved — is read and parsed and is still no reload. What
    /// `.2` and `.3` are told about is a config that says something new, and
    /// this is the arm that keeps a cosmetic edit from waking them.
    #[test]
    fn a_config_saying_the_same_in_different_words_is_no_reload() {
        let path = a_config_file("recommented", ORBITAL, 100);
        let (mut reload, parsed) = watching(path.clone(), ORBITAL);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        written(&path, &format!("# the one project\n{ORBITAL}"), 200);

        assert_eq!(reload.checks(at(4)), Reloaded::Unchanged);
        assert_eq!(
            parsed.get(),
            2,
            "the file was parsed; it just said nothing new"
        );
    }

    /// **The case a check comparing timestamps skips for ever.** One config
    /// put in place of another under the stamp the file already had — two
    /// writes inside one of the kernel's coarse ticks, a `cp -p`, an
    /// unpacked archive — and a check that reads the stamp reads *nothing
    /// has been written* about a file the reader has genuinely changed. Not
    /// once: from then on, since the stamp it holds is the stamp the file
    /// now carries.
    #[test]
    fn a_config_put_there_under_the_stamp_the_file_already_had_is_still_read() {
        let path = a_config_file("same-stamp", ORBITAL, 100);
        let (mut reload, _) = watching(path.clone(), ORBITAL);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        written(&path, ORBITAL_AND_FERRY, 100);

        assert_eq!(came_into_force(reload.checks(at(4))), &both());
    }

    /// The same file put back under a stamp *earlier* than the one it had,
    /// which is what a restore from a backup gives. A check asking whether
    /// the file is newer would never read it.
    #[test]
    fn a_config_put_there_under_an_earlier_stamp_is_still_read() {
        let path = a_config_file("earlier-stamp", ORBITAL, 100);
        let (mut reload, _) = watching(path.clone(), ORBITAL);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        written(&path, ORBITAL_AND_FERRY, 50);

        assert_eq!(came_into_force(reload.checks(at(4))), &both());
    }

    /// The rule with teeth: the running config stands. Said by putting the
    /// file back to exactly what was in force and getting `Unchanged` —
    /// which a reload that had fallen back to defaults, or taken the
    /// half-parsed text, could not give.
    #[test]
    fn a_config_that_will_not_parse_leaves_the_running_one_in_force() {
        let path = a_config_file("unparsed", ORBITAL_AND_FERRY, 100);
        let (mut reload, _) = watching(path.clone(), ORBITAL_AND_FERRY);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        written(&path, NOT_TOML, 200);
        assert_eq!(reload.checks(at(4)), Reloaded::Broken);

        written(&path, ORBITAL_AND_FERRY, 300);
        assert_eq!(
            reload.checks(at(6)),
            Reloaded::Unchanged,
            "the config in force is the one the file said before it was broken"
        );
    }

    #[test]
    fn a_config_that_will_not_open_leaves_the_running_one_in_force() {
        let path = a_config_file("unopened", ORBITAL_AND_FERRY, 100);
        let (mut reload, _) = watching(path.clone(), ORBITAL_AND_FERRY);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        std::fs::remove_file(&path).expect("the config is ours to remove");
        assert_eq!(reload.checks(at(4)), Reloaded::Broken);

        written(&path, ORBITAL_AND_FERRY, 300);
        assert_eq!(reload.checks(at(6)), Reloaded::Unchanged);
    }

    /// The same, out of a broken config rather than a good one: a reader who
    /// fixes the file is believed however it is stamped. This is the state a
    /// reader can otherwise reach and never leave — they have edited the
    /// config back to something that works and `bdi` goes on saying it will
    /// not load.
    #[test]
    fn a_config_fixed_under_the_stamp_the_broken_one_had_is_still_read() {
        let path = a_config_file("fixed-same-stamp", ORBITAL, 100);
        let (mut reload, parsed) = watching(path.clone(), ORBITAL);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        written(&path, NOT_TOML, 200);
        assert_eq!(reload.checks(at(4)), Reloaded::Broken);

        written(&path, ORBITAL_AND_FERRY, 200);
        assert_eq!(came_into_force(reload.checks(at(6))), &both());
        assert_eq!(parsed.get(), 3);
    }

    /// A file still saying what it said last time is `Untouched` whether or
    /// not that text parses, and the notice stays up on the strength of it:
    /// nothing new has happened to a config that is still broken. Parsing it
    /// again every couple of seconds to say so would be work for an answer
    /// already given.
    #[test]
    fn a_config_still_broken_is_not_parsed_again_to_say_so() {
        let path = a_config_file("still-broken", ORBITAL, 100);
        let (mut reload, parsed) = watching(path.clone(), ORBITAL);
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);

        written(&path, NOT_TOML, 200);
        assert_eq!(reload.checks(at(4)), Reloaded::Broken);

        assert_eq!(reload.checks(at(6)), Reloaded::Untouched);
        assert_eq!(parsed.get(), 2);
    }

    #[test]
    fn a_check_that_is_not_due_reads_nothing() {
        let path = a_config_file("not-due", ORBITAL, 100);
        let (mut reload, parsed) = watching(path, ORBITAL);

        assert_eq!(reload.checks(at(1)), Reloaded::Untouched);
        assert_eq!(parsed.get(), 0);
    }

    #[test]
    fn a_check_falls_due_its_interval_after_the_one_before_it() {
        let path = a_config_file("interval", ORBITAL, 100);
        let (mut reload, _) = watching(path, ORBITAL);

        assert_eq!(reload.checks_in(at(0)), Some(EVERY_TWO_SECONDS));
        assert_eq!(reload.checks_in(at(1)), Some(Duration::from_secs(1)));
        assert_eq!(reload.checks(at(2)), Reloaded::Unchanged);
        assert_eq!(reload.checks_in(at(2)), Some(EVERY_TWO_SECONDS));
    }

    /// A check due in the past is due now rather than a negative duration
    /// away: the loop sleeps for what this says, and `Duration` has no
    /// negative.
    #[test]
    fn a_check_already_overdue_is_due_now() {
        let path = a_config_file("overdue", ORBITAL, 100);
        let (reload, _) = watching(path, ORBITAL);

        assert_eq!(reload.checks_in(at(9)), Some(Duration::ZERO));
    }

    /// An interval too long to reach the end of time arms nothing, as
    /// `due_after` says — and a reload that never falls due is one the loop
    /// never wakes for.
    #[test]
    fn an_interval_that_outruns_time_falls_due_never() {
        let path = a_config_file("outruns", ORBITAL, 100);
        let mut reload = Reload::watching(
            path,
            Duration::MAX,
            a_config(ORBITAL),
            Box::new(Config::from_toml),
            at(0),
        );

        assert_eq!(reload.checks_in(at(0)), None);
        assert_eq!(reload.checks(at(9)), Reloaded::Untouched);
    }
}

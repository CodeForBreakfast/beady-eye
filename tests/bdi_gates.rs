//! `bdi gates` closes the gate on a merged pull request, and goes on looking
//! through a GitHub that refuses it and a tracker that does not answer,
//! reporting each.
//!
//! The cases run the binary against the `bd` and `gh` shims, because what is
//! under test is the process a supervisor starts and leaves running.

mod terminal;

use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use terminal::die_with;
use terminal::shims::ShimmedTracker;

/// Long enough for a look that is coming to have come, and short enough
/// that one that is not is a failure rather than a hang.
const GIVING_UP: Duration = Duration::from_secs(10);

const GATE_LIST: &str = include_str!("fixtures/bd_1.3.0_gate_list.json");
const MERGED: &str = include_str!("fixtures/gh_2.102.0_pr_view_merged.json");
const OPEN: &str = include_str!("fixtures/gh_2.102.0_pr_view_open.json");

/// Each gh:pr gate in `GATE_LIST`, beside what `bd dep list` answers for the
/// beads it holds back. ark-0i5 waits on example/ark#42 and ark-eb1 on
/// example/ark#7.
const HELD_BACK: [(&str, &str); 4] = [
    (
        "ark-0i5",
        include_str!("fixtures/bd_1.3.0_dep_list_up_ark-0i5.json"),
    ),
    (
        "ark-eb1",
        include_str!("fixtures/bd_1.3.0_dep_list_up_ark-eb1.json"),
    ),
    (
        "ark-6pp",
        include_str!("fixtures/bd_1.3.0_dep_list_up_ark-6pp.json"),
    ),
    (
        "ark-tg0",
        include_str!("fixtures/bd_1.3.0_dep_list_up_ark-tg0.json"),
    ),
];

const RESOLVING_42: &str = "gate resolve ark-0i5 --reason Pull request example/ark#42 merged \
                            as 5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff.";

fn viewed(number: u64) -> String {
    format!("pr view {number} --repo example/ark --json state,mergeCommit")
}

/// A home holding a config that names `projects`, each a directory of its
/// own under it, settling every owner's gates once a second.
fn a_home_naming(named: &str, projects: &[&str]) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-gates-{named}-{}", std::process::id()));
    let mut config = String::from("[gates]\npoll_seconds = 1\n");
    for project in projects {
        let path = home.join(project);
        std::fs::create_dir_all(&path).expect("the directory is ours to make");
        config.push_str(&format!(
            "\n[[projects]]\nname = \"{project}\"\npath = \"{}\"\n",
            path.display()
        ));
    }
    std::fs::write(home.join("config.toml"), config).expect("the config is ours to write");
    home
}

/// The tracker at `project` holding the captured gates.
fn holds_the_captured_gates(tracker: &ShimmedTracker, project: &str) {
    tracker.answers_for(project, "gate list --limit 0 --json", GATE_LIST);
    for (gate, held) in HELD_BACK {
        tracker.answers_for(
            project,
            &format!("dep list {gate} --direction=up --type blocks --json"),
            held,
        );
    }
}

/// A `gh` answering from files under `home`.
struct ShimmedGitHub {
    answers: PathBuf,
    called: PathBuf,
}

impl ShimmedGitHub {
    fn beside(home: &Path) -> Self {
        Self {
            answers: home.join("gh-answers"),
            called: home.join("gh-called"),
        }
    }

    fn answers_with(&self, asked: &str, text: &str) {
        let answer = self.answers.join(asked);
        std::fs::create_dir_all(answer.parent().expect("an answer sits in a directory"))
            .expect("the answers are ours to write");
        std::fs::write(answer, text).expect("the answer is ours to write");
    }

    fn refuses_with(&self, asked: &str, said: &str) {
        self.answers_with(&format!("{asked}.refused"), said);
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.called)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn environment(&self) -> [(&'static str, PathBuf); 2] {
        [
            ("BDI_SHIM_GH_ANSWERS", self.answers.clone()),
            ("BDI_SHIM_GH_CALLED", self.called.clone()),
        ]
    }
}

/// A running `bdi gates` that is killed and reaped when the test ends, with
/// its stdout in a file the test reads as it goes.
struct Settling {
    child: Child,
    said: PathBuf,
}

impl Settling {
    fn started(home: &Path, tracker: &ShimmedTracker, github: &ShimmedGitHub) -> Self {
        let said = home.join("said");
        let spawned_by = std::process::id();
        let mut command = Command::new(env!("CARGO_BIN_EXE_bdi"));
        command
            .args(["gates", "--config"])
            .arg(home.join("config.toml"))
            .current_dir(home)
            .env("HOME", home)
            .env_remove("BEADS_DIR")
            .env_remove("BEADS_DOLT_PASSWORD")
            .env_remove("BDI_PROJECT")
            .envs(tracker.environment())
            .envs(github.environment())
            .stdout(File::create(&said).expect("the file is ours to make"));
        // A test binary that is killed runs no `Drop`, so the kernel is asked
        // to end `bdi gates` with its spawner.
        unsafe { command.pre_exec(move || die_with(spawned_by)) };
        Self {
            child: command.spawn().expect("bdi gates starts"),
            said,
        }
    }

    fn said(&self) -> String {
        std::fs::read_to_string(&self.said).unwrap_or_default()
    }

    /// Wait until `bdi gates` has said `line`, and fail with all it said if
    /// it never does.
    #[track_caller]
    fn says(&self, line: &str) {
        until(
            || self.said().lines().any(|said| said == line),
            &format!("{line:?} among {:?}", self.said()),
        );
    }
}

impl Drop for Settling {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[track_caller]
fn until(holds: impl Fn() -> bool, awaited: &str) {
    let giving_up = Instant::now() + GIVING_UP;
    while !holds() {
        assert!(Instant::now() < giving_up, "never saw {awaited}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_merged_pull_request_closes_the_gate_waiting_on_it() {
    let home = a_home_naming("merged", &["arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "✓ Gate resolved: ark-0i5\n");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&viewed(42), MERGED);
    github.answers_with(&viewed(7), OPEN);

    let settling = Settling::started(&home, &tracker, &github);

    settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
    assert!(
        tracker.calls().iter().any(|call| call == RESOLVING_42),
        "{:?}",
        tracker.calls()
    );
}

/// GitHub answers a pull request in an organisation whose single sign-on
/// authorisation has lapsed with a refusal, and gh exits non-zero until
/// someone runs `gh auth refresh`. The words are not a capture, and bdi
/// drops them: what a refusal is read as is decided by the exit.
#[test]
fn a_github_refusing_for_lapsed_single_sign_on_is_reported_closes_nothing_and_is_asked_again() {
    let home = a_home_naming("refused", &["arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    let github = ShimmedGitHub::beside(&home);
    github.refuses_with(
        &viewed(42),
        "GraphQL: Resource protected by organization SAML enforcement. You must grant your \
         OAuth token access to this organization. (repository)\n",
    );
    github.answers_with(&viewed(7), OPEN);

    let settling = Settling::started(&home, &tracker, &github);

    settling.says(
        "example/ark#42: GitHub did not say where it stands, so no gate waiting on it was \
         touched: gh exited 1 for a reason bdi cannot place",
    );
    until(
        || {
            github
                .calls()
                .iter()
                .filter(|call| **call == viewed(42))
                .count()
                >= 2
        },
        "a second look at example/ark#42",
    );
    assert!(
        tracker
            .calls()
            .iter()
            .all(|call| !call.starts_with("gate resolve")),
        "{:?}",
        tracker.calls()
    );
}

/// The dunwich tracker has no answers written down, so the shim refuses
/// every call to it as a tracker that is not there does.
#[test]
fn a_tracker_that_does_not_answer_is_reported_and_the_others_are_settled() {
    let home = a_home_naming("unanswered", &["dunwich", "arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&viewed(42), MERGED);
    github.answers_with(&viewed(7), OPEN);

    let settling = Settling::started(&home, &tracker, &github);

    settling.says("dunwich: its gh:pr gates could not be read: the tracker did not answer");
    settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
}

//! `bdi gates` closes the gate on a merged pull request, a closed issue or a
//! release published since the gate was made, and goes on looking
//! through a GitHub that refuses it and a tracker that does not answer,
//! reporting each. Listening, it settles the pull request a delivery signed
//! with its secret names, and refuses every other. Given a socket, it reads a
//! tracker a producer covers only once the producer says it changed, and
//! closes a bead gate as soon as the tracker holding its bead changes.
//!
//! The cases run the binary against the `bd` and `gh` shims, because what is
//! under test is the process a supervisor starts and leaves running.

mod terminal;

use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use beady_eye::collect::pr_events::{self, EVENTS};
use chrono::{TimeDelta, Timelike, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use terminal::die_with;
use terminal::shims::ShimmedTracker;

/// Long enough for a look that is coming to have come, and short enough
/// that one that is not is a failure rather than a hang.
const GIVING_UP: Duration = Duration::from_secs(10);

const GATE_LIST: &str = include_str!("fixtures/bd_1.3.0_gate_list.json");
const MERGED: &str = r#"{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}"#;
const OPEN: &str = r#"{"state":"OPEN","isDraft":false,"mergeCommit":null}"#;

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

/// The query a delivery about #`number` asks.
fn viewed(number: u64) -> String {
    format!(
        "api graphql -f owner=example -f name=ark -f query=query($owner:String!,$name:String!)\
         {{repository(owner:$owner,name:$name){{pr{number}:pullRequest(number:{number})\
         {{state {}}}}}}}",
        pr_events::fields(EVENTS)
    )
}

/// GitHub's answer to [`viewed`]: #`number` with `fields`.
fn answer(number: u64, fields: &str) -> String {
    format!(r#"{{"data":{{"repository":{{"pr{number}":{fields}}}}}}}"#)
}

/// #7 open and #42 merged, as the one query a look asks reads them.
const QUERIED: &str = include_str!("fixtures/gh_2.102.0_api_graphql_ark_42_merged.json");
const QUERIED_OPEN: &str = include_str!("fixtures/gh_2.102.0_api_graphql_ark_open.json");

/// The one query a look asks about #7 and #42 in example/ark.
fn queried() -> String {
    format!(
        "api graphql -f owner=example -f name=ark -f query=query($owner:String!,$name:String!)\
         {{repository(owner:$owner,name:$name){{pr7:pullRequest(number:7){{state {fields}}} \
         pr42:pullRequest(number:42){{state {fields}}}}}}}",
        fields = pr_events::fields(EVENTS)
    )
}

/// A home holding a config that names `projects`, each a directory of its
/// own under it, settling every owner's gates once a second.
fn a_home_naming(named: &str, projects: &[&str]) -> PathBuf {
    a_home_looking_every(named, projects, 1)
}

/// A home as [`a_home_naming`] gives, looking every `seconds`.
fn a_home_looking_every(named: &str, projects: &[&str], seconds: u64) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-gates-{named}-{}", std::process::id()));
    let mut config = format!("[gates]\npoll_seconds = {seconds}\n");
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

    /// The file the shim answers `asked` from. The shim finds it by the call
    /// written beside it, so no path grows with the call: macOS refuses a
    /// path over 1024 bytes, and the query a look asks is longer than that.
    fn file_for(&self, asked: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        asked.hash(&mut hasher);
        self.answers.join(format!("{:016x}", hasher.finish()))
    }

    fn answers_with(&self, asked: &str, text: &str) {
        let answer = self.file_for(asked);
        std::fs::create_dir_all(&self.answers).expect("the answers are ours to write");
        std::fs::write(&answer, text).expect("the answer is ours to write");
        std::fs::write(answer.with_extension("asked"), asked).expect("the call is ours to write");
    }

    fn refuses_with(&self, asked: &str, said: &str) {
        self.answers_with(&format!("{asked}.refused"), said);
    }

    fn stops_refusing(&self, asked: &str) {
        let refusal = self.file_for(&format!("{asked}.refused"));
        std::fs::remove_file(refusal.with_extension("asked")).expect("the call was ours to write");
        std::fs::remove_file(refusal).expect("the refusal was ours to write");
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
        Self::spawned(home, gates(home, tracker, github))
    }

    /// Started listening on a port of the system's choosing, with
    /// [`SECRET`] in the environment.
    fn listening(home: &Path, tracker: &ShimmedTracker, github: &ShimmedGitHub) -> Self {
        let mut command = gates(home, tracker, github);
        command
            .args(["--listen", "127.0.0.1:0"])
            .env("BDI_GATES_WEBHOOK_SECRET", SECRET);
        Self::spawned(home, command)
    }

    fn spawned(home: &Path, mut command: Command) -> Self {
        let said = home.join("said");
        command.stdout(File::create(&said).expect("the file is ours to make"));
        let spawned_by = std::process::id();
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

    /// The address it said it takes deliveries on.
    fn address(&self) -> SocketAddr {
        const TAKING: &str = "GitHub's deliveries to ";
        until(|| self.said().contains(TAKING), "the address it listens on");
        let said = self.said();
        let after = &said[said.find(TAKING).expect("it said") + TAKING.len()..];
        after
            .split([',', '\n'])
            .next()
            .and_then(|address| address.parse().ok())
            .unwrap_or_else(|| panic!("an address in {said:?}"))
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

/// `bdi gates` for the home, answered by the shims.
fn gates(home: &Path, tracker: &ShimmedTracker, github: &ShimmedGitHub) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bdi"));
    command
        .args(["gates", "--config"])
        .arg(home.join("config.toml"))
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .env_remove("BDI_GATES_WEBHOOK_SECRET")
        .envs(tracker.environment())
        .envs(github.environment());
    command
}

const SECRET: &str = "swordfish";

/// A pull_request delivery for example/ark#42, cut down to what bdi reads
/// and a little of what it does not.
const DELIVERED_42: &str = r#"{"action":"closed","number":42,"pull_request":{"merged":true},"repository":{"full_name":"example/ark"}}"#;

/// `body` signed as GitHub signs it with `secret`.
fn signed(secret: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key");
    mac.update(body.as_bytes());
    let digest: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256={digest}")
}

/// The status `bdi gates` answers `request` with.
fn answered(address: SocketAddr, request: &str) -> u16 {
    let mut stream = TcpStream::connect(address).expect("bdi gates takes the connection");
    stream
        .set_read_timeout(Some(GIVING_UP))
        .expect("a timeout is ours to set");
    stream
        .write_all(request.as_bytes())
        .expect("the request is sent");
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .expect("bdi gates answers");
    answer
        .split(' ')
        .nth(1)
        .and_then(|status| status.parse().ok())
        .unwrap_or_else(|| panic!("a status line in {answer:?}"))
}

/// The status a delivery of `event` carrying `body` is answered with.
fn delivered(address: SocketAddr, event: &str, signature: Option<&str>, body: &str) -> u16 {
    let signature = signature
        .map(|signature| format!("X-Hub-Signature-256: {signature}\r\n"))
        .unwrap_or_default();
    answered(
        address,
        &format!(
            "POST /hook HTTP/1.1\r\nHost: bdi\r\nConnection: close\r\nContent-Type: \
             application/json\r\nContent-Length: {}\r\nX-GitHub-Event: \
             {event}\r\n{signature}\r\n{body}",
            body.len()
        ),
    )
}

#[track_caller]
fn until(holds: impl Fn() -> bool, awaited: &str) {
    let giving_up = Instant::now() + GIVING_UP;
    while !holds() {
        assert!(Instant::now() < giving_up, "never saw {awaited}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The look asks GitHub about both pull requests the captured gates wait on
/// in the one call.
#[test]
fn a_merged_pull_request_closes_the_gate_waiting_on_it() {
    let home = a_home_looking_every("merged", &["arkham"], 3600);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "✓ Gate resolved: ark-0i5\n");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&queried(), QUERIED);

    let settling = Settling::started(&home, &tracker, &github);

    settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
    assert!(
        tracker.calls().iter().any(|call| call == RESOLVING_42),
        "{:?}",
        tracker.calls()
    );
    assert_eq!(github.calls(), [queried()]);
}

/// ark-zl0 waits on example/ark#31 closing, and ark-8dq on example/ark
/// publishing a release after it was made, on 2026-10-12.
const UPSTREAM: &str = include_str!("fixtures/bd_1.3.0_gate_list_upstream.json");

const ISSUE_31: &str = "api graphql -f owner=example -f name=ark -f \
                        query=query($owner:String!,$name:String!){repository(owner:$owner,\
                        name:$name){issue(number:31){state}}}";

const RELEASES: &str = "api graphql -f owner=example -f name=ark -f \
                        query=query($owner:String!,$name:String!){repository(owner:$owner,\
                        name:$name){releases(first:10,orderBy:{field:CREATED_AT,direction:\
                        DESC}){nodes{tagName isDraft publishedAt}}}}";

/// What gh 2.102.0 prints, exit 1, when GitHub refuses a GraphQL query for
/// the primary rate limit. The user id is invented.
const RATE_LIMITED: &str = "GraphQL: API rate limit already exceeded for user ID 1234567.\n";

/// `gh api rate_limit` with the GraphQL limit spent until 2100-01-01, far
/// enough off that no look in the test can come after it.
const GRAPHQL_SPENT_UNTIL_2100: &str = r#"{"resources":{"core":{"limit":5000,"used":31,"remaining":4969,"reset":4102444800},"graphql":{"limit":5000,"used":5000,"remaining":0,"reset":4102444800}},"rate":{"limit":5000,"used":31,"remaining":4969,"reset":4102444800}}"#;

const WAITING_UNTIL_2100: &str = "example/ark#7: GitHub refused it for the rate limit of the \
                                  login gh runs as, so GitHub is asked nothing more until the \
                                  limit resets at 2100-01-01 00:00:00 UTC";

/// A home whose gates wait on example/ark#7 and #42, with a GitHub that
/// refuses the query about them for a rate limit spent until 2100.
fn rate_limited(named: &str) -> (PathBuf, ShimmedTracker, ShimmedGitHub) {
    let home = a_home_naming(named, &["arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    let github = ShimmedGitHub::beside(&home);
    github.refuses_with(&queried(), RATE_LIMITED);
    github.answers_with("api rate_limit", GRAPHQL_SPENT_UNTIL_2100);
    (home, tracker, github)
}

/// The poll is a second, so a second and a half without a second look is the
/// limit being waited out rather than a look that has not come yet.
#[test]
fn a_look_github_refuses_for_a_rate_limit_waits_until_the_limit_resets_and_says_so() {
    let (home, tracker, github) = rate_limited("rate-limited");

    let settling = Settling::started(&home, &tracker, &github);

    settling.says(WAITING_UNTIL_2100);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        github.calls(),
        [queried(), "api rate_limit".to_string()],
        "#42 is never asked after, and no look follows the first"
    );
}

#[test]
fn a_delivery_while_a_rate_limit_is_waited_out_asks_github_nothing() {
    let (home, tracker, github) = rate_limited("rate-limited-delivery");

    let settling = Settling::listening(&home, &tracker, &github);
    let address = settling.address();
    settling.says(WAITING_UNTIL_2100);

    assert_eq!(
        delivered(
            address,
            "pull_request",
            Some(&signed(SECRET, DELIVERED_42)),
            DELIVERED_42
        ),
        202
    );
    settling.says(
        "example/ark#42: a delivery came while GitHub's rate limit is waited out, so the next \
         look settles it",
    );
    assert_eq!(github.calls(), [queried(), "api rate_limit".to_string()]);
}

const LISTING_GATES: &str = "gate list --limit 0 --json";

/// A producer connected to the socket `bdi gates` was told to take reports
/// on.
struct Producer {
    writing: UnixStream,
    answers: BufReader<UnixStream>,
}

impl Producer {
    fn connected(at: &Path) -> Self {
        until(|| at.exists(), "the socket bdi gates takes reports on");
        let writing = UnixStream::connect(at).expect("bdi gates takes the connection");
        writing
            .set_read_timeout(Some(GIVING_UP))
            .expect("a timeout is ours to set");
        let answers = BufReader::new(writing.try_clone().expect("the stream clones"));
        Self { writing, answers }
    }

    /// Say `line`, and what `bdi gates` answered.
    fn says(&mut self, line: &str) -> String {
        writeln!(self.writing, "{line}").expect("the line is sent");
        let mut answer = String::new();
        self.answers
            .read_line(&mut answer)
            .expect("bdi gates answers");
        answer.trim_end().to_string()
    }
}

/// How many times `github` has been asked the query a look asks about the
/// captured gates, which is how many looks there have been.
fn looks(github: &ShimmedGitHub) -> usize {
    github
        .calls()
        .iter()
        .filter(|call| **call == queried())
        .count()
}

/// Once a producer covers arkham, looks go on asking GitHub and read no bd, so
/// the gate closed there is the one thing that makes the next look read it.
#[test]
fn a_tracker_a_producer_covers_is_read_again_after_a_gate_is_closed_in_it() {
    let home = a_home_naming("covered-closed", &["arkham"]);
    let socket = home.join("changes.sock");
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&queried(), QUERIED_OPEN);
    let mut command = gates(&home, &tracker, &github);
    command.arg("--socket").arg(&socket);
    let _settling = Settling::spawned(&home, command);
    let listings = || {
        tracker
            .calls()
            .iter()
            .filter(|call| *call == LISTING_GATES)
            .count()
    };
    until(|| looks(&github) == 1, "the first look");
    let mut producer = Producer::connected(&socket);
    assert_eq!(producer.says("covered arkham"), "ok arkham");
    until(|| looks(&github) == 2, "a look after the cover");
    assert_eq!(listings(), 1);

    tracker.answers_for("arkham", LISTING_GATES, UPSTREAM);
    let closing_zl0 = "gate resolve ark-zl0 --reason example/ark#31 closed.";
    tracker.answers_for("arkham", closing_zl0, "✓ Gate resolved: ark-zl0\n");
    github.answers_with(
        ISSUE_31,
        r#"{"data":{"repository":{"issue":{"state":"CLOSED"}}}}"#,
    );
    github.answers_with(
        RELEASES,
        r#"{"data":{"repository":{"releases":{"nodes":[]}}}}"#,
    );
    assert_eq!(producer.says("arkham"), "ok arkham");
    until(
        || tracker.calls().iter().any(|call| call == closing_zl0),
        "the gate written in arkham closed",
    );

    let read_for_the_close = listings();
    until(
        || listings() > read_for_the_close,
        "arkham read again after bdi gates closed a gate in it",
    );
}

/// arkham's config says nothing of `poll`, so without `--no-poll` each look
/// would read it again.
#[test]
fn bdi_gates_with_no_poll_reads_a_tracker_once_and_goes_on_asking_github() {
    let home = a_home_naming("no-poll", &["arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&queried(), QUERIED_OPEN);
    let mut command = gates(&home, &tracker, &github);
    command.arg("--no-poll");
    let _settling = Settling::spawned(&home, command);

    until(|| looks(&github) >= 3, "three looks");

    let listings = tracker
        .calls()
        .iter()
        .filter(|call| *call == LISTING_GATES)
        .count();
    assert_eq!(listings, 1);
}

/// ark-06g, a timer gate of two hours made on 2026-10-12 at 07:32:28.
const TIMER: &str = include_str!("fixtures/bd_1.3.0_gate_list_timer.json");

/// The close of ark-06g, whose timer ran out at `deadline`.
fn closing_06g(deadline: &str) -> String {
    format!("gate resolve ark-06g --reason Its timer ran out at {deadline} UTC.")
}

/// Nothing but the deadline wakes `bdi gates` here: it looks once an hour,
/// and the producer covering arkham reports no change.
#[test]
fn a_covered_tracker_s_timer_gate_closes_at_its_deadline_with_no_read_of_bd_meanwhile() {
    let home = a_home_looking_every("timer-covered", &["arkham"], 3600);
    let socket = home.join("changes.sock");
    let tracker = ShimmedTracker::beside(&home);
    let deadline = Utc::now()
        .with_nanosecond(0)
        .expect("a whole second is a time")
        + TimeDelta::seconds(3);
    let made = (deadline - TimeDelta::hours(2)).format("%Y-%m-%dT%H:%M:%SZ");
    tracker.answers_for(
        "arkham",
        LISTING_GATES,
        &TIMER.replace("2026-10-12T07:32:28Z", &made.to_string()),
    );
    let closing = closing_06g(&deadline.format("%Y-%m-%d %H:%M:%S").to_string());
    tracker.answers_for("arkham", &closing, "✓ Gate resolved: ark-06g\n");
    let github = ShimmedGitHub::beside(&home);
    let mut command = gates(&home, &tracker, &github);
    command.arg("--socket").arg(&socket);
    let _settling = Settling::spawned(&home, command);
    until(
        || tracker.calls().iter().any(|call| call == LISTING_GATES),
        "the first look's read of arkham",
    );
    let mut producer = Producer::connected(&socket);
    assert_eq!(producer.says("covered arkham"), "ok arkham");

    until(|| tracker.calls().contains(&closing), "ark-06g closed");

    let late = Utc::now() - deadline;
    assert!(
        late < TimeDelta::seconds(3),
        "closed {late} after its deadline"
    );
    assert_eq!(tracker.calls(), [LISTING_GATES.to_string(), closing]);
}

/// ark-689, a bead gate in arkham awaiting dun-ski in dunwich.
const BEAD_GATE: &str = include_str!("fixtures/bd_1.3.0_gate_list_bead.json");
const NO_GATES: &str = include_str!("fixtures/bd_1.3.0_gate_list_none.json");
const AWAITING_DUN_SKI: &str =
    "list --id dun-ski --all --include-gates --include-infra --include-templates --limit 0 --json";

/// Nothing but the producer wakes `bdi gates` here: it looks once an hour,
/// and the producer covering both trackers says dunwich changed only once
/// dun-ski has closed.
#[test]
fn a_bead_gate_closes_moments_after_its_bead_in_another_tracker_closes_with_no_read_of_bd_meanwhile(
) {
    let home = a_home_looking_every("bead", &["arkham", "dunwich"], 3600);
    let config = home.join("config.toml");
    let stated = std::fs::read_to_string(&config).expect("the config was ours to write")
        + "prefix = \"dun\"\n";
    std::fs::write(&config, stated).expect("the config is ours to write");
    let socket = home.join("changes.sock");
    let tracker = ShimmedTracker::beside(&home);
    tracker.answers_for("arkham", LISTING_GATES, BEAD_GATE);
    tracker.answers_for("dunwich", LISTING_GATES, NO_GATES);
    tracker.answers_for(
        "dunwich",
        AWAITING_DUN_SKI,
        include_str!("fixtures/bd_1.3.0_list_id_dun-ski_open.json"),
    );
    let closing = "gate resolve ark-689 --reason Bead dun-ski in dunwich closed.";
    tracker.answers_for("arkham", closing, "✓ Gate resolved: ark-689\n");
    let github = ShimmedGitHub::beside(&home);
    let mut command = gates(&home, &tracker, &github);
    command.arg("--socket").arg(&socket);
    let settling = Settling::spawned(&home, command);
    until(
        || tracker.calls_for("dunwich").len() == 2,
        "the first look's reads of dunwich",
    );
    let mut producer = Producer::connected(&socket);
    assert_eq!(producer.says("covered arkham"), "ok arkham");
    assert_eq!(producer.says("covered dunwich"), "ok dunwich");
    // The shim records a call before it answers it, so nothing observable
    // says the first look's last read is done.
    std::thread::sleep(Duration::from_secs(1));
    let read_before = tracker.calls();

    tracker.answers_for(
        "dunwich",
        AWAITING_DUN_SKI,
        include_str!("fixtures/bd_1.3.0_list_id_dun-ski_closed.json"),
    );
    let told = Instant::now();
    assert_eq!(producer.says("dunwich"), "ok dunwich");
    settling.says("Bead dun-ski in dunwich closed: arkham closed gate ark-689");

    assert!(
        told.elapsed() < Duration::from_secs(3),
        "closed {:?} after the change",
        told.elapsed()
    );
    assert_eq!(
        tracker.calls()[read_before.len()..],
        [LISTING_GATES, AWAITING_DUN_SKI, closing]
    );
    assert_eq!(
        tracker.calls()[..read_before.len()],
        [LISTING_GATES, LISTING_GATES, AWAITING_DUN_SKI]
    );
}

#[test]
fn a_signed_delivery_settles_the_pull_request_it_names_between_looks() {
    let home = a_home_looking_every("delivered", &["arkham"], 3600);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "✓ Gate resolved: ark-0i5\n");
    let github = ShimmedGitHub::beside(&home);
    github.refuses_with(&queried(), "HTTP 502\n");
    github.answers_with(&viewed(42), &answer(42, MERGED));

    let settling = Settling::listening(&home, &tracker, &github);
    let address = settling.address();
    settling.says(
        "example/ark#42: GitHub did not say where it stands, so no gate waiting on it was \
         touched: gh exited 1 for a reason bdi cannot place",
    );

    assert_eq!(
        delivered(
            address,
            "pull_request",
            Some(&signed(SECRET, DELIVERED_42)),
            DELIVERED_42
        ),
        202
    );
    settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
    assert_eq!(github.calls(), [queried(), viewed(42)]);
}

/// The producer's report reaches `bdi gates` while it waits for a delivery,
/// so the delivery that follows is the first thing it settles after the
/// report.
#[test]
fn a_delivery_reads_a_tracker_a_producer_said_changed_just_before_it() {
    let home = a_home_looking_every("delivered-after-report", &["arkham"], 3600);
    let socket = home.join("changes.sock");
    let tracker = ShimmedTracker::beside(&home);
    tracker.answers_for(
        "arkham",
        LISTING_GATES,
        include_str!("fixtures/bd_1.3.0_gate_list_none.json"),
    );
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&viewed(42), &answer(42, MERGED));
    let mut command = gates(&home, &tracker, &github);
    command
        .args(["--listen", "127.0.0.1:0"])
        .env("BDI_GATES_WEBHOOK_SECRET", SECRET)
        .arg("--socket")
        .arg(&socket);
    let settling = Settling::spawned(&home, command);
    let address = settling.address();
    let mut producer = Producer::connected(&socket);
    assert_eq!(producer.says("covered arkham"), "ok arkham");
    until(|| !tracker.calls().is_empty(), "the first look");

    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "✓ Gate resolved: ark-0i5\n");
    assert_eq!(producer.says("arkham"), "ok arkham");
    assert_eq!(
        delivered(
            address,
            "pull_request",
            Some(&signed(SECRET, DELIVERED_42)),
            DELIVERED_42
        ),
        202
    );

    settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
}

const SHA: &str = "5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff";

/// What GitHub lists of a commit: #42 open with it as head, #43 open without
/// it, #44 closed with it.
const LISTED_FOR_SHA: &str = r#"[
    {"number":42,"state":"open","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}},
    {"number":43,"state":"open","head":{"sha":"0badc0de0badc0de0badc0de0badc0de0badc0de"}},
    {"number":44,"state":"closed","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}
]"#;

/// The checks on a commit finishing settles the open pull requests it heads,
/// whichever event says so.
#[test]
fn a_signed_check_suite_or_status_delivery_settles_the_open_pull_requests_the_commit_heads() {
    let check_suite = format!(
        r#"{{"action":"completed","check_suite":{{"head_sha":"{SHA}"}},"repository":{{"full_name":"example/ark"}}}}"#
    );
    let status = format!(
        r#"{{"state":"success","sha":"{SHA}","repository":{{"full_name":"example/ark"}}}}"#
    );
    for (event, body) in [("check_suite", check_suite), ("status", status)] {
        let home = a_home_looking_every(&format!("checks-{event}"), &["arkham"], 3600);
        let tracker = ShimmedTracker::beside(&home);
        holds_the_captured_gates(&tracker, "arkham");
        tracker.answers_for("arkham", RESOLVING_42, "✓ Gate resolved: ark-0i5\n");
        let github = ShimmedGitHub::beside(&home);
        github.refuses_with(&queried(), "HTTP 502\n");
        github.answers_with(
            &format!("api repos/example/ark/commits/{SHA}/pulls?per_page=100"),
            LISTED_FOR_SHA,
        );
        github.answers_with(&viewed(42), &answer(42, MERGED));

        let settling = Settling::listening(&home, &tracker, &github);
        let address = settling.address();
        settling.says(
            "example/ark#42: GitHub did not say where it stands, so no gate waiting on it was \
             touched: gh exited 1 for a reason bdi cannot place",
        );

        assert_eq!(
            delivered(address, event, Some(&signed(SECRET, &body)), &body),
            202
        );
        settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
        assert_eq!(
            github.calls(),
            [
                queried(),
                format!("api repos/example/ark/commits/{SHA}/pulls?per_page=100"),
                viewed(42)
            ],
            "{event}"
        );
    }
}

/// A delivery that is refused, or about anything but a pull request, never
/// reaches GitHub.
#[test]
fn only_a_delivery_signed_with_the_secret_is_taken_and_only_a_pull_request_one_is_acted_on() {
    let listening = Listening::after_its_first_look("refusing");
    let address = listening.address;

    let ping = r#"{"zen":"Keep it logically awesome.","hook_id":1}"#;
    assert_eq!(
        delivered(address, "ping", Some(&signed(SECRET, ping)), ping),
        202
    );
    assert_eq!(
        delivered(
            address,
            "pull_request",
            Some(&signed("hunter2", DELIVERED_42)),
            DELIVERED_42
        ),
        401
    );
    assert_eq!(delivered(address, "pull_request", None, DELIVERED_42), 401);
    listening
        .settling
        .says("a delivery was refused: its X-Hub-Signature-256 is not the secret's");
    listening
        .settling
        .says("a delivery was refused: it carries no X-Hub-Signature-256");
    listening.caught_up();
    assert_eq!(listening.github.calls(), [queried(), viewed(7)]);
}

#[test]
fn a_listening_bdi_gates_answers_a_readiness_probe() {
    let home = a_home_looking_every("healthy", &["arkham"], 3600);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&queried(), QUERIED_OPEN);

    let settling = Settling::listening(&home, &tracker, &github);

    assert_eq!(answered(settling.address(), PROBED), 200);
}

const PROBED: &str = "GET /healthz HTTP/1.1\r\nHost: bdi\r\nConnection: close\r\n\r\n";

/// GitHub refuses the query a look asks about both pull requests, as it does
/// for a token that has expired, and then answers it again.
#[test]
fn a_readiness_probe_fails_while_every_read_of_github_is_refused_and_passes_once_one_is_not() {
    let home = a_home_naming("unhealthy", &["arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&queried(), QUERIED_OPEN);
    github.refuses_with(&queried(), "HTTP 401: Bad credentials\n");

    let settling = Settling::listening(&home, &tracker, &github);
    let address = settling.address();
    until(
        || answered(address, PROBED) == 503,
        "the probe failing while GitHub refuses every read",
    );

    github.stops_refusing(&queried());
    until(
        || answered(address, PROBED) == 200,
        "the probe passing once GitHub answers a read",
    );
}

/// A listening `bdi gates` whose first look has asked GitHub about both pull
/// requests the captured gates wait on, each still open, and settled nothing.
struct Listening {
    github: ShimmedGitHub,
    settling: Settling,
    address: SocketAddr,
    _tracker: ShimmedTracker,
}

impl Listening {
    fn after_its_first_look(named: &str) -> Self {
        let home = a_home_looking_every(named, &["arkham"], 3600);
        let tracker = ShimmedTracker::beside(&home);
        holds_the_captured_gates(&tracker, "arkham");
        let github = ShimmedGitHub::beside(&home);
        github.answers_with(&queried(), QUERIED_OPEN);
        github.answers_with(&viewed(42), &answer(42, OPEN));
        github.answers_with(&viewed(7), &answer(7, OPEN));
        let settling = Settling::listening(&home, &tracker, &github);
        let address = settling.address();
        until(|| github.calls().len() == 1, "the first look");
        Self {
            github,
            settling,
            address,
            _tracker: tracker,
        }
    }

    /// Wait until every delivery taken before now has been settled. They are
    /// settled one at a time in the order they came, so GitHub being asked
    /// about a delivery for #7 means all those before it have been, and #7
    /// joins [`ShimmedGitHub::calls`].
    fn caught_up(&self) {
        let asked = || {
            self.github
                .calls()
                .iter()
                .filter(|call| **call == viewed(7))
                .count()
        };
        let before = asked();
        assert_eq!(
            delivered(
                self.address,
                "pull_request",
                Some(&signed(SECRET, DELIVERED_7)),
                DELIVERED_7
            ),
            202
        );
        until(|| asked() > before, "GitHub asked about #7");
    }
}

/// A pull_request delivery for example/ark#7, which the captured gates wait
/// on and which is open.
const DELIVERED_7: &str =
    r#"{"action":"edited","number":7,"repository":{"full_name":"example/ark"}}"#;

/// The `gh` shim refuses a call made with the secret's variable in its
/// environment, so a secret handed on would leave #42 unsettled.
#[test]
fn the_secret_reaches_no_program_bdi_gates_starts_even_where_a_file_gives_it() {
    let home = a_home_looking_every("secret", &["arkham"], 3600);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "✓ Gate resolved: ark-0i5\n");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&queried(), QUERIED);
    github.answers_with(&viewed(42), &answer(42, MERGED));
    let file = home.join("secret");
    std::fs::write(&file, format!("{SECRET}\n")).expect("the file is ours to write");

    let mut command = gates(&home, &tracker, &github);
    command
        .args(["--listen", "127.0.0.1:0", "--webhook-secret-file"])
        .arg(&file)
        .env("BDI_GATES_WEBHOOK_SECRET", "not the one in the file");
    let settling = Settling::spawned(&home, command);

    settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
    assert_eq!(
        delivered(
            settling.address(),
            "pull_request",
            Some(&signed(SECRET, DELIVERED_42)),
            DELIVERED_42
        ),
        202,
        "the file's secret is the one taken"
    );
}

#[test]
fn listening_with_no_secret_refuses_to_start() {
    let home = a_home_naming("secretless", &["arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    let github = ShimmedGitHub::beside(&home);

    let out = gates(&home, &tracker, &github)
        .args(["--listen", "127.0.0.1:0"])
        .output()
        .expect("bdi gates runs");

    assert!(!out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Error: --listen needs the secret GitHub signs deliveries with, from \
         --webhook-secret-file or BDI_GATES_WEBHOOK_SECRET\n"
    );
    assert_eq!(tracker.calls(), Vec::<String>::new());
}

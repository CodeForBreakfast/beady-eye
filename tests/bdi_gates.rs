//! `bdi gates` closes the gate on a merged pull request, and goes on looking
//! through a GitHub that refuses it and a tracker that does not answer,
//! reporting each. Listening, it settles the pull request a delivery signed
//! with its secret names, and refuses every other.
//!
//! The cases run the binary against the `bd` and `gh` shims, because what is
//! under test is the process a supervisor starts and leaves running.

mod terminal;

use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use sha2::Sha256;
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
    format!("pr view {number} --repo example/ark --json state,isDraft,mergeCommit")
}

/// #7 open and #42 merged, as the one query a look asks reads them.
const QUERIED: &str = include_str!("fixtures/gh_2.102.0_api_graphql_ark_42_merged.json");
const QUERIED_OPEN: &str = include_str!("fixtures/gh_2.102.0_api_graphql_ark_open.json");

/// The one query a look asks about #7 and #42 in example/ark.
fn queried() -> String {
    "api graphql -f owner=example -f name=ark -f query=query($owner:String!,$name:String!)\
     {repository(owner:$owner,name:$name){pr7:pullRequest(number:7){state isDraft mergeCommit{oid}} \
     pr42:pullRequest(number:42){state isDraft mergeCommit{oid}}}}"
        .to_string()
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

    fn answers_with(&self, asked: &str, text: &str) {
        let answer = self.answers.join(asked);
        std::fs::create_dir_all(answer.parent().expect("an answer sits in a directory"))
            .expect("the answers are ours to write");
        std::fs::write(answer, text).expect("the answer is ours to write");
    }

    fn refuses_with(&self, asked: &str, said: &str) {
        self.answers_with(&format!("{asked}.refused"), said);
    }

    fn stops_refusing(&self, asked: &str) {
        std::fs::remove_file(self.answers.join(format!("{asked}.refused")))
            .expect("the refusal was ours to write");
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
            .lines()
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
        &queried(),
        "GraphQL: Resource protected by organization SAML enforcement. You must grant your \
         OAuth token access to this organization. (repository)\n",
    );

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
                .filter(|call| **call == queried())
                .count()
                >= 2
        },
        "a second look at example/ark",
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

/// The poll is a second, so three seconds without a second look is the
/// limit being waited out rather than a look that has not come yet.
#[test]
fn a_look_github_refuses_for_a_rate_limit_waits_until_the_limit_resets_and_says_so() {
    let (home, tracker, github) = rate_limited("rate-limited");

    let settling = Settling::started(&home, &tracker, &github);

    settling.says(WAITING_UNTIL_2100);
    std::thread::sleep(Duration::from_secs(3));
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

/// The dunwich tracker has no answers written down, so the shim refuses
/// every call to it as a tracker that is not there does.
#[test]
fn a_tracker_that_does_not_answer_is_reported_and_the_others_are_settled() {
    let home = a_home_naming("unanswered", &["dunwich", "arkham"]);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "");
    let github = ShimmedGitHub::beside(&home);
    github.answers_with(&queried(), QUERIED);

    let settling = Settling::started(&home, &tracker, &github);

    settling.says("dunwich: its gh:pr gates could not be read: the tracker did not answer");
    settling.says("example/ark#42 merged: arkham closed gate ark-0i5");
}

#[test]
fn a_signed_delivery_settles_the_pull_request_it_names_between_looks() {
    let home = a_home_looking_every("delivered", &["arkham"], 3600);
    let tracker = ShimmedTracker::beside(&home);
    holds_the_captured_gates(&tracker, "arkham");
    tracker.answers_for("arkham", RESOLVING_42, "✓ Gate resolved: ark-0i5\n");
    let github = ShimmedGitHub::beside(&home);
    github.refuses_with(&queried(), "HTTP 502\n");
    github.answers_with(&viewed(42), MERGED);

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
        github.answers_with(&viewed(42), OPEN);
        github.answers_with(&viewed(7), OPEN);
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

/// [`DELIVERED_42`] with spaces after it to make `size` bytes, which JSON
/// reads as the same delivery.
fn padded(size: usize) -> String {
    DELIVERED_42.to_string() + &" ".repeat(size - DELIVERED_42.len())
}

/// A body at the limit is read and settled. One byte over is refused for its
/// size, though it is signed with the secret, and GitHub is never asked.
#[test]
fn a_signed_delivery_over_a_mebibyte_is_refused_and_settles_nothing() {
    let listening = Listening::after_its_first_look("large");
    let at_the_limit = padded(1024 * 1024);
    let over_it = padded(1024 * 1024 + 1);

    assert_eq!(
        delivered(
            listening.address,
            "pull_request",
            Some(&signed(SECRET, &at_the_limit)),
            &at_the_limit
        ),
        202
    );
    listening.caught_up();
    assert_eq!(listening.github.calls(), [queried(), viewed(42), viewed(7)]);

    assert_eq!(
        delivered(
            listening.address,
            "pull_request",
            Some(&signed(SECRET, &over_it)),
            &over_it
        ),
        413
    );
    listening
        .settling
        .says("a delivery was refused: it is larger than any pull_request delivery");
    listening.caught_up();
    assert_eq!(
        listening.github.calls(),
        [queried(), viewed(42), viewed(7), viewed(7)]
    );
}

/// The body sent is the whole of a signed delivery, so reading to the end of
/// what arrived, rather than to the length it gives, would settle it.
#[test]
fn a_signed_delivery_ending_before_its_length_is_closed_unanswered_and_settles_nothing() {
    let listening = Listening::after_its_first_look("short");
    let mut stream = TcpStream::connect(listening.address).expect("bdi gates takes it");
    stream
        .set_read_timeout(Some(GIVING_UP))
        .expect("a timeout is ours to set");
    stream
        .write_all(
            format!(
                "POST /hook HTTP/1.1\r\nHost: bdi\r\nContent-Length: {}\r\nX-GitHub-Event: \
                 pull_request\r\nX-Hub-Signature-256: {}\r\n\r\n{DELIVERED_42}",
                DELIVERED_42.len() + 1,
                signed(SECRET, DELIVERED_42)
            )
            .as_bytes(),
        )
        .expect("the request is sent");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("the sending side is ours to close");

    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .expect("bdi gates closes the connection");
    assert_eq!(answer, "");
    listening.caught_up();
    assert_eq!(listening.github.calls(), [queried(), viewed(7)]);
}

/// Eight senders that give their headers and never their body hold every
/// answer there is, so a ninth request, signed with the secret, is turned
/// away and never reaches GitHub. Its body is as large as a delivery may be,
/// so it is still being sent when the answer comes. Once they go, deliveries
/// are taken again.
#[test]
fn a_signed_delivery_beyond_eight_at_once_is_turned_away_and_settles_nothing() {
    let listening = Listening::after_its_first_look("busy");
    let stalled: Vec<TcpStream> = (0..8)
        .map(|_| {
            let mut stream = TcpStream::connect(listening.address).expect("bdi gates takes it");
            stream
                .write_all(
                    b"POST /hook HTTP/1.1\r\nHost: bdi\r\nContent-Length: 2048\r\nX-GitHub-Event: \
                      pull_request\r\n\r\n",
                )
                .expect("the headers are sent");
            stream
        })
        .collect();

    let largest = padded(1024 * 1024);
    let ninth = || {
        delivered(
            listening.address,
            "pull_request",
            Some(&signed(SECRET, &largest)),
            &largest,
        )
    };
    assert_eq!(ninth(), 503);
    drop(stalled);
    until(
        || delivered(listening.address, "pull_request", None, DELIVERED_42) == 401,
        "a request answered once the eight have gone",
    );
    listening.caught_up();
    assert_eq!(listening.github.calls(), [queried(), viewed(7)]);

    assert_eq!(ninth(), 202);
    listening.caught_up();
    assert_eq!(
        listening.github.calls(),
        [queried(), viewed(7), viewed(42), viewed(7)]
    );
}

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
    github.answers_with(&viewed(42), MERGED);
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

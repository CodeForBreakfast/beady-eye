//! An open gate with no open blocker is as ready as any other bead, in the
//! listing `bdi --beads` writes and on its row of the tree.
//!
//! A bare `bd ready` leaves gates out, as work nobody claims, so the canned
//! tracker answers the two questions apart the way bd does: the bare one with
//! no gate, and the one asked for gates with the gates that have no open
//! blocker. Both are read through bd's own adapter, because that is what asks
//! the second question.

use beady_eye::collect::bd;
use beady_eye::collect::herdr::Herdr;
use beady_eye::config::Config;
use beady_eye::model::snapshot::{Filter, Listing, Only};
use serde_json::Value;

use crate::canned::Canned;

/// Three beads each blocked by a `gh:pr` gate, as bd 1.3.0 wrote them.
const GH_PR_GATES: &str = include_str!("../fixtures/bd_1.3.0_gh_pr_gates.json");

/// What bd says is ready when asked for gates: all three, since none of
/// them waits on anything.
const READY_GATES: &str = r#"[
  {"id":"dun-w8h","title":"Gate: gh:pr","status":"open","issue_type":"gate"},
  {"id":"dun-5s6","title":"Gate: gh:pr","status":"open","issue_type":"gate"},
  {"id":"dun-auk","title":"Gate: gh:pr","status":"open","issue_type":"gate"}
]"#;

const ONE_PROJECT: &str = r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"
"#;

/// The tracker holding the gates, answering every question `bdi` asks of it
/// as bd would, and a herdr holding no pane. Every bead in the capture is
/// open, so the listings of unfinished work hold all of them too.
fn the_tracker() -> Canned {
    [
        ("list --all --include-gates --limit 0 --json", GH_PR_GATES),
        ("list --include-gates --limit 0 --json", GH_PR_GATES),
        (
            "list --all --include-gates --brief --limit 0 --json",
            GH_PR_GATES,
        ),
        ("query ephemeral=true --all --limit 0 --json", "[]"),
        ("ready --brief --limit 0 --json", "[]"),
        ("ready --type gate --brief --limit 0 --json", READY_GATES),
        ("blocked --json", "[]"),
    ]
    .into_iter()
    .fold(
        Canned::default().herdr_holding(r#"{"result":{"agents":[]}}"#),
        |canned, (asked, answer)| {
            canned.answering(
                &format!("bd -C /srv/work/dunwich --readonly {asked}"),
                answer,
            )
        },
    )
}

fn ready<'a>(beads: impl IntoIterator<Item = &'a Value>, id: &str) -> &'a Value {
    &beads
        .into_iter()
        .find(|bead| bead["id"] == id)
        .unwrap_or_else(|| panic!("{id} is listed"))["ready"]
}

#[test]
fn an_open_gate_with_no_open_blocker_is_ready_in_the_listing_and_on_its_row() {
    let cfg = Config::from_toml(ONE_PROJECT).expect("the config parses");
    let now = "2026-08-30T12:00:00Z".parse().expect("the instant parses");
    let runner = the_tracker();

    let unfinished = beady_eye::app::run(
        &cfg,
        &Herdr::new(&runner),
        &bd::Cli::new(&runner).for_unfinished_work(),
        Filter::All,
        now,
    );
    let listing = serde_json::to_value(Listing::of(&unfinished, &Only::default(), false))
        .expect("serialises");
    let forest = serde_json::to_value(beady_eye::app::run(
        &cfg,
        &Herdr::new(&runner),
        &bd::Cli::new(&runner),
        Filter::All,
        now,
    ))
    .expect("the snapshot serialises");

    let listed = listing["beads"].as_array().expect("beads");
    let rows: Vec<&Value> = forest["trees"]
        .as_array()
        .expect("trees")
        .iter()
        .flat_map(|tree| tree["nodes"].as_array().expect("nodes"))
        .collect();
    for (id, is_ready) in [("dun-w8h", true), ("dun-5s6", true), ("dun-4ga", false)] {
        assert_eq!(ready(listed, id), is_ready, "{id} in the listing");
        assert_eq!(ready(rows.iter().copied(), id), is_ready, "{id} on its row");
    }
}

//! A forest with nothing in it, and the lines it still draws.

use super::*;
use pretty_assertions::assert_eq;

/// A pane working in a configured project, with no tracker answering for
/// it, so it reaches the forest as a loose one.
const WORKING_IN_DUNWICH: &str = r#"{"result":{"agents":[
  {"pane_id":"w:p1","cwd":"/srv/work/dunwich","agent_status":"working"}
]}}"#;

/// A pane working in a directory no configured project covers.
const WORKING_NOWHERE: &str = r#"{"result":{"agents":[
  {"pane_id":"w:pF","cwd":"/srv/spike","agent_status":"idle"}
]}}"#;

fn one_pane(json: &str) -> Vec<Pane> {
    parse_agent_list(A_SESSION, json).expect("the panes parse")
}

/// A snapshot built out of exactly what it is handed, with no other
/// project's trees or panes standing behind it.
///
/// Everything the forest draws a line from arrives through one of these:
/// the trees and failed projects `Collected` carries, and the panes a
/// join resolves. Handed one of them alone the forest draws that one
/// thing, which is what it takes to ask whether anything else is quietly
/// supplying a line.
fn only(collected: Collected, panes: &[Pane]) -> Snapshot {
    let cfg = cfg();
    let joined = join::resolve(&[], Listed::all(panes), &cfg);
    snapshot::build(
        Collected {
            read_at: every_project_read(),
            ..collected
        },
        panes,
        &joined,
        &cfg,
        a_provider(ProviderState::Answering),
        Filter::LiveAgents,
        now(),
    )
}

fn says_it_holds_nothing(forest: &Forest) -> bool {
    forest
        .lines()
        .iter()
        .any(|line| matches!(line.content, Content::Note(Note::NoRoots)))
}

/// The first frame of a run. Nothing has been read, so every project is
/// drawn from the name the config gave it and holds nothing yet — which
/// is the point: the reader sees the shape of their work in the time it
/// takes to draw a frame, rather than a blank terminal for as long as the
/// trackers take.
#[test]
fn a_run_that_has_read_nothing_yet_draws_a_line_for_every_configured_project() {
    let awaiting = Snapshot::awaiting(
        vec!["dunwich".to_string(), "ferry".to_string()],
        Vec::new(),
        A_PROVIDER,
        Scope::Everything,
        Filter::LiveAgents,
        now(),
    );

    assert_eq!(sketch(&flatten(awaiting)), vec!["▾ dunwich", "▾ ferry"]);
}

/// A scope the reader did not type is said on the screen: a reader who
/// sees one project could think the others vanished. It is the last
/// line, below the groups, where the hidden trees say what the filter
/// took away.
#[test]
fn a_scope_the_directory_chose_is_said_below_the_groups() {
    let chosen = Snapshot {
        scope: Scope::Directory {
            project: "dunwich".to_string(),
            widened: Vec::new(),
        },
        ..built(Filter::LiveAgents)
    };

    let drawn = sketch(&flatten(chosen));

    assert_eq!(
        drawn.last().map(String::as_str),
        Some("  ~ reading dunwich")
    );
    let last_group = drawn.iter().rposition(|line| line.contains('['));
    assert!(
        last_group.is_some_and(|at| at + 1 < drawn.len()),
        "the line is not below the groups: {drawn:#?}"
    );
}

/// Said from the first frame, before any tracker has answered: the
/// projects the run is about are on the screen, and so is why.
#[test]
fn the_first_frame_already_says_the_directory_chose() {
    let chosen = Snapshot::awaiting(
        vec!["dunwich".to_string()],
        Vec::new(),
        A_PROVIDER,
        Scope::Directory {
            project: "dunwich".to_string(),
            widened: Vec::new(),
        },
        Filter::LiveAgents,
        now(),
    );

    assert_eq!(
        sketch(&flatten(chosen)),
        vec!["▾ dunwich", "  ~ reading dunwich"]
    );
}

/// Scoping by `--project` is silent because the reader typed it, and a
/// run reading everything has nothing to say.
#[test]
fn a_scope_the_reader_typed_is_silent() {
    for scope in [Scope::Everything, Scope::Asked(vec!["dunwich".to_string()])] {
        let awaiting = Snapshot::awaiting(
            vec!["dunwich".to_string()],
            Vec::new(),
            A_PROVIDER,
            scope,
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(sketch(&flatten(awaiting)), vec!["▾ dunwich"]);
    }
}

/// A project whose tracker answered and held nothing draws no line, as it
/// always has.
///
/// Only a project nothing has read is drawn empty, and `read_at` is the
/// whole of what tells the two apart. Without it a tracker that answered
/// with no roots would sit there looking forever about to produce some,
/// and the forest would never say what it says here instead.
#[test]
fn a_project_read_and_holding_nothing_draws_no_line() {
    let read = Snapshot {
        read_at: std::collections::BTreeMap::from([("dunwich".to_string(), now())]),
        ..Snapshot::awaiting(
            vec!["dunwich".to_string()],
            Vec::new(),
            A_PROVIDER,
            Scope::Everything,
            Filter::LiveAgents,
            now(),
        )
    };

    assert_eq!(sketch(&flatten(read)), vec!["! NoRoots"]);
}

/// Every tree a snapshot holds belongs to a project it names.
///
/// The forest walks the projects and takes the trees that follow each
/// one, so a tree whose project is missing from the list is a tree that
/// vanishes off the screen with nothing said. Both come from the same
/// `cfg.projects` inside `build`, which is what makes it true — and this
/// is what says so, because nothing about the types does.
#[test]
fn every_tree_belongs_to_a_project_the_snapshot_names() {
    let snapshot = built(Filter::All);

    for tree in &snapshot.trees {
        assert!(
            snapshot.projects.contains(&tree.project),
            "{} is not among {:?}",
            tree.project,
            snapshot.projects
        );
    }
    // The config's three, and not the fixture's fourth: `lunar` is a
    // failed project no `[[projects]]` entry names, so it is reported
    // among the failures and has no line of its own to be drawn on.
    assert_eq!(snapshot.projects, ["dunwich", "ferry", "harbour"]);
}

/// Every tracker answered and none of them had a root. The screen has to
/// carry the reason, because a pane drawn blank reads as a crash.
#[test]
fn a_forest_with_nothing_in_it_says_so_rather_than_drawing_nothing() {
    let forest = flatten(only(Collected::default(), &[]));

    assert_eq!(sketch(&forest), vec!["! NoRoots"]);
}

/// The fold keys reach a forest that holds nothing, and a walk over it is
/// given a count of rounds taken from beads there are none of. It has no
/// fold to point either way, so the screen does not move and still says
/// what it holds.
#[test]
fn the_fold_keys_on_a_forest_with_nothing_in_it_leave_it_saying_so() {
    for action in [Action::ExpandSubtree, Action::CollapseSubtree] {
        let mut forest = flatten(only(Collected::default(), &[]));

        assert!(!forest.apply(action), "{:#?}", sketch(&forest));
        assert_eq!(sketch(&forest), vec!["! NoRoots"]);
    }
}

/// Everything that can stand alone in a forest. A conflict is not among
/// them: a pane can only conflict over a bead a tracker answered for, so
/// it never arrives without the tree that bead is in.
#[test]
fn a_forest_holding_any_one_thing_does_not_say_it_holds_nothing() {
    let cases = [
        (
            "a tree",
            only(
                Collected {
                    trees: vec![tree_of("dunwich", DUNWICH)],
                    failed_projects: Vec::new(),
                    read_at: every_project_read(),
                    speaks_until: BTreeMap::new(),
                    read_for_reach: BTreeSet::new(),
                },
                &[],
            ),
        ),
        (
            "a hidden tree",
            only(
                Collected {
                    trees: vec![tree_of("harbour", HARBOUR)],
                    failed_projects: Vec::new(),
                    read_at: every_project_read(),
                    speaks_until: BTreeMap::new(),
                    read_for_reach: BTreeSet::new(),
                },
                &[],
            ),
        ),
        (
            "a failed project",
            only(
                Collected {
                    trees: Vec::new(),
                    failed_projects: vec![FailedProject {
                        project: "lunar".into(),
                        tracker: TrackerFailure::Unstartable,
                    }],
                    read_at: every_project_read(),
                    speaks_until: BTreeMap::new(),
                    read_for_reach: BTreeSet::new(),
                },
                &[],
            ),
        ),
        (
            "a loose pane",
            only(Collected::default(), &one_pane(WORKING_IN_DUNWICH)),
        ),
        (
            "an unconfigured pane",
            only(Collected::default(), &one_pane(WORKING_NOWHERE)),
        ),
    ];

    for (held, snapshot) in cases {
        let forest = flatten(snapshot);

        assert!(
            forest.lines().len() > 0,
            "a forest holding {held} drew nothing at all"
        );
        assert!(
            !says_it_holds_nothing(&forest),
            "a forest holding {held} said it holds nothing: {:?}",
            sketch(&forest)
        );
    }
}

#[test]
fn a_forest_holding_trees_and_every_group_does_not_say_it_holds_nothing() {
    assert!(!says_it_holds_nothing(&flatten(snapshot())));
}

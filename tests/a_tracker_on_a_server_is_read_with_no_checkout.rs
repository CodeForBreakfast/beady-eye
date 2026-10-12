//! A host holding none of a project's checkouts reads its tracker from the
//! server it sits on, and lists what a host reading it through a checkout
//! lists.
//!
//! The binary runs, because what is under test crosses every layer: the
//! config naming a database, the directory `bdi` keeps for it under `HOME`,
//! and bd asked there.

mod terminal;

use std::path::Path;
use std::process::{Command, Output};

use terminal::shims::ShimmedTracker;

/// What `bd list --all --json` said about this repository's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

fn a_home_configured_with(named: &str, config: &str) -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(home.join(".config/beady-eye/config.toml"), config)
        .expect("the config is ours to write");
    home
}

fn bdi_beads(home: &Path, environment: &[(String, String)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .args(["--beads", "--all-projects"])
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BDI_PROJECT")
        .envs(environment.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .output()
        .expect("bdi runs")
}

fn listed(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "bdi exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let mut listing: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("bdi wrote a listing");
    listing
        .as_object_mut()
        .expect("a listing is an object")
        .remove("generated_at")
        .expect("and says when it was made");
    listing
}

#[test]
fn a_tracker_named_by_its_database_lists_what_its_checkout_lists() {
    let checked_out = a_home_configured_with("server-checkout", "");
    std::fs::create_dir_all(checked_out.join("arkham")).expect("the checkout is ours to make");
    std::fs::write(
        checked_out.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"arkham\"\npath = \"{}\"\n",
            checked_out.join("arkham").display()
        ),
    )
    .expect("the config is ours to write");
    let on_a_server = a_home_configured_with(
        "server-database",
        "[dolt]\nhost = \"db.example.invalid\"\n\n[[projects]]\nname = \"arkham\"\ndatabase = \"arkham\"\n",
    );
    let read = |home: &Path| {
        let bd = ShimmedTracker::beside(home);
        bd.holds_for("arkham", THE_TRACKER);
        listed(&bdi_beads(home, &bd.environment()))
    };

    let through_the_checkout = read(&checked_out);
    let through_the_server = read(&on_a_server);

    assert!(
        through_the_checkout["beads"]
            .as_array()
            .is_some_and(|beads| !beads.is_empty()),
        "the checkout listed nothing to compare with: {through_the_checkout}"
    );
    assert_eq!(through_the_server, through_the_checkout);
    let metadata = on_a_server.join(".cache/beady-eye/trackers/arkham/.beads/metadata.json");
    assert!(
        metadata.exists(),
        "bd was asked where bdi keeps no tracker: {}",
        metadata.display()
    );

    std::fs::remove_dir_all(checked_out).expect("the home is ours to remove");
    std::fs::remove_dir_all(on_a_server).expect("the home is ours to remove");
}

use anyhow::Context;

use crate::model::types::Bead;

/// Parse the output of `bd dep tree <root> --direction=up --json`.
///
/// bd returns a flat array already in its own render order, each row carrying
/// `parent_id` and `edge_from_parent`. We keep the rows and re-order them
/// ourselves; see `model::tree`.
pub fn parse_dep_tree(s: &str) -> anyhow::Result<Vec<Bead>> {
    serde_json::from_str(s).context("bd dep tree --json returned a shape we do not understand")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::{Edge, Status};

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");

    fn fixture() -> Vec<Bead> {
        parse_dep_tree(FIXTURE).expect("the captured tree parses")
    }

    fn row(id: &str) -> Bead {
        fixture()
            .into_iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("{id} is in the fixture"))
    }

    #[test]
    fn parses_every_row() {
        assert_eq!(fixture().len(), 6);
    }

    #[test]
    fn the_root_has_no_parent_and_children_carry_their_edge() {
        let root = row("bdi-3um");
        assert_eq!(root.parent_id, None);
        assert_eq!(root.edge_from_parent, None);

        let child = row("bdi-3um.10");
        assert_eq!(child.parent_id.as_deref(), Some("bdi-3um"));
        assert_eq!(child.edge_from_parent, Some(Edge::ParentChild));

        let blocker = row("bdi-3um.11");
        assert_eq!(blocker.parent_id.as_deref(), Some("bdi-3um.10"));
        assert_eq!(blocker.edge_from_parent, Some(Edge::Blocks));
    }

    #[test]
    fn statuses_map_onto_the_enum() {
        assert_eq!(row("bdi-3um").status, Status::InProgress);
        assert_eq!(row("bdi-3um.10").status, Status::Open);
        assert_eq!(row("bdi-3um.1").status, Status::Closed);
    }

    #[test]
    fn every_status_spelling_bd_writes_is_recognised() {
        let spellings = ["open", "in_progress", "blocked", "closed", "deferred"];
        let expected = [
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Closed,
            Status::Deferred,
        ];

        for (spelling, want) in spellings.iter().zip(expected) {
            let json = format!(r#"[{{"id":"x","title":"t","status":"{spelling}"}}]"#);
            assert_eq!(parse_dep_tree(&json).unwrap()[0].status, want);
        }
    }

    #[test]
    fn a_status_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"marinating"}]"#;
        let beads = parse_dep_tree(json).expect("an unknown status still parses");
        assert_eq!(beads[0].status, Status::Other("marinating".to_string()));
    }

    #[test]
    fn an_edge_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"open","edge_from_parent":"discovered-by"}]"#;
        let beads = parse_dep_tree(json).expect("an unknown edge still parses");
        assert_eq!(
            beads[0].edge_from_parent,
            Some(Edge::Other("discovered-by".to_string()))
        );
    }

    #[test]
    fn metadata_is_carried_inline_and_absent_metadata_is_an_empty_map() {
        let carrying = row("bdi-3um.3");
        assert_eq!(
            carrying.metadata.get("working_topic").map(String::as_str),
            Some("beady-eye/core-json-bdi-3um.3")
        );

        assert!(row("bdi-3um.10").metadata.is_empty());
    }

    #[test]
    fn the_timestamps_the_age_rules_need_follow_the_row() {
        let closed = row("bdi-3um.1");
        assert!(closed.started_at.is_some());
        assert!(closed.closed_at.is_some());

        let open = row("bdi-3um.10");
        assert_eq!(open.started_at, None);
        assert_eq!(open.closed_at, None);
        assert!(open.updated_at.is_some());
    }

    #[test]
    fn an_unclaimed_bead_has_no_assignee() {
        assert_eq!(row("bdi-3um.3").assignee.as_deref(), Some("Graeme Foster"));
        assert_eq!(row("bdi-3um.10").assignee, None);
    }

    #[test]
    fn issue_type_distinguishes_the_root_epic_from_its_tasks() {
        assert_eq!(row("bdi-3um").issue_type, "epic");
        assert_eq!(row("bdi-3um.10").issue_type, "task");
    }

    #[test]
    fn truncation_survives_the_parse() {
        for bead in fixture() {
            assert!(!bead.truncated, "{} is not truncated", bead.id);
        }

        let json = r#"[{"id":"x","title":"t","status":"open","truncated":true}]"#;
        assert!(parse_dep_tree(json).unwrap()[0].truncated);
    }

    #[test]
    fn a_wrongly_typed_field_is_an_error_not_a_default() {
        let bad = r#"[{"id":"x","title":"t","status":"open","priority":"high"}]"#;
        assert!(parse_dep_tree(bad).is_err());
    }

    #[test]
    fn work_in_flight_ranks_ahead_of_work_that_is_finished() {
        let mut statuses = vec![
            Status::Closed,
            Status::Open,
            Status::Other("marinating".to_string()),
            Status::InProgress,
            Status::Deferred,
            Status::Blocked,
        ];
        statuses.sort_by_key(Status::rank);

        assert_eq!(
            statuses,
            vec![
                Status::InProgress,
                Status::Blocked,
                Status::Open,
                Status::Deferred,
                Status::Closed,
                Status::Other("marinating".to_string()),
            ]
        );
        assert!(Status::Closed.is_closed());
        assert!(!Status::Open.is_closed());
    }
}

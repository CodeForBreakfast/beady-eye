//! The short text a row carries beside its title, for a metadata key a setup
//! asked `bdi` to show.
//!
//! Nothing here learns what a key means. Which keys are worth drawing, and
//! what to say for a value, are both config's to state; this renders what it
//! is given and knows no more about `blocked_on` than about any other key.

use serde::Serialize;

use crate::config::{Badge, Colour};
use crate::model::types::Bead;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Badged {
    pub key: String,
    pub text: String,
    /// Where the badge points, for one whose config names a `link`.
    ///
    /// Beside the text rather than inside it, and beside it the whole way to
    /// the row: a line is fitted by the visible width of what its spans say,
    /// so a URL held in the text would be counted in the columns the row has
    /// to spend.
    pub link: Option<String>,
    /// What to draw it in, for one whose config named a colour. A slot the
    /// view resolves rather than a colour, so nothing here learns what the
    /// row is drawn in any more than it learns what the key means.
    pub colour: Option<Colour>,
}

/// Render the configured badges that apply to this bead.
pub fn badges_for(bead: &Bead, badges: &[Badge]) -> Vec<Badged> {
    badges
        .iter()
        .filter_map(|b| {
            let value = bead.metadata.get(&b.key)?;
            let text = b.apply(value)?;
            Some(Badged {
                key: b.key.clone(),
                text,
                link: b.link_for(value),
                colour: b.colour,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use pretty_assertions::assert_eq;

    use crate::config::Pattern;

    fn matching(pattern: &str) -> Pattern {
        Pattern::new(pattern).expect("the pattern compiles")
    }

    fn bead_with(metadata: &str) -> Bead {
        let json =
            format!(r#"[{{"id":"p-1","title":"root","status":"open","metadata":{metadata}}}]"#);
        parse_beads(&json).expect("the bead parses").remove(0)
    }

    #[test]
    fn badges_render_only_where_the_key_and_match_agree() {
        let bead = bead_with(r#"{"blocked_on":"human","delivery_pr":"owner/repo#7"}"#);
        let cfg = vec![
            Badge {
                key: "blocked_on".into(),
                match_value: Some(matching("human")),
                render: "waiting".into(),
                link: None,
                colour: None,
            },
            Badge {
                key: "blocked_on".into(),
                match_value: Some(matching("dependency")),
                render: "dep".into(),
                link: None,
                colour: None,
            },
            Badge {
                key: "absent_key".into(),
                match_value: None,
                render: "never".into(),
                link: None,
                colour: None,
            },
        ];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got,
            vec![Badged {
                key: "blocked_on".to_string(),
                text: "waiting".to_string(),
                link: None,
                colour: None,
            }]
        );
    }

    /// Nothing in the model learns what a metadata key means: a key it has
    /// never heard of renders exactly as well as a familiar one.
    #[test]
    fn badges_render_a_configured_key_without_interpreting_it() {
        let bead = bead_with(r#"{"xyzzy":"plugh"}"#);
        let cfg = vec![Badge {
            key: "xyzzy".into(),
            match_value: None,
            render: "→ {}".into(),
            link: None,
            colour: None,
        }];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got,
            vec![Badged {
                key: "xyzzy".to_string(),
                text: "→ plugh".to_string(),
                link: None,
                colour: None,
            }]
        );
    }

    /// The colour a badge's config named travels beside its text, because
    /// nothing downstream of here can go back and read the config: the row
    /// carries badges and the view draws a row.
    #[test]
    fn a_badges_colour_travels_with_its_text() {
        let bead = bead_with(r#"{"jira":"ATLAS-19"}"#);
        let cfg = vec![Badge {
            key: "jira".into(),
            match_value: None,
            render: "{}".into(),
            link: None,
            colour: Some(Colour::Status),
        }];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got,
            vec![Badged {
                key: "jira".to_string(),
                text: "ATLAS-19".to_string(),
                link: None,
                colour: Some(Colour::Status),
            }]
        );
    }

    #[test]
    fn a_bead_with_no_configured_badges_renders_none() {
        let bead = bead_with(r#"{"blocked_on":"human"}"#);

        assert_eq!(badges_for(&bead, &[]), vec![]);
    }
}

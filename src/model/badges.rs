//! The short text a row carries beside its title, for a metadata key a setup
//! asked `bdi` to show.
//!
//! Nothing here learns what a key means. Which keys are worth drawing, and
//! what to say for a value, are both config's to state; this renders what it
//! is given and knows no more about `blocked_on` than about any other key.

use serde::Serialize;

use crate::config::Badge;
use crate::model::types::Bead;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Badged {
    pub key: String,
    pub text: String,
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
            },
            Badge {
                key: "blocked_on".into(),
                match_value: Some(matching("dependency")),
                render: "dep".into(),
            },
            Badge {
                key: "absent_key".into(),
                match_value: None,
                render: "never".into(),
            },
        ];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got,
            vec![Badged {
                key: "blocked_on".to_string(),
                text: "waiting".to_string(),
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
        }];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got,
            vec![Badged {
                key: "xyzzy".to_string(),
                text: "→ plugh".to_string(),
            }]
        );
    }

    #[test]
    fn a_bead_with_no_configured_badges_renders_none() {
        let bead = bead_with(r#"{"blocked_on":"human"}"#);

        assert_eq!(badges_for(&bead, &[]), vec![]);
    }
}

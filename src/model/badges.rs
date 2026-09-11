//! The short text a row carries beside its title, for a value a setup
//! asked `bdi` to show.
//!
//! Nothing here learns what a key means. Which keys are worth drawing, and
//! what to say for a value, are both config's to state; this renders what it
//! is given and knows no more about `blocked_on` than about any other key.

use serde::Serialize;

use crate::config::{Badge, BadgeKey, Colour};
use crate::model::types::Bead;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Badged {
    pub key: String,
    pub text: String,
    /// What it says instead on a row too narrow for `text`, for one whose
    /// config names a `short`.
    ///
    /// Both forms travel, because which of them a row can afford is the width
    /// the row was given, and nothing here has one.
    pub short: Option<String>,
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

/// A badge that drew less than its config asked for.
///
/// A badge with no `link` is a filter: it is written to decline, so a value
/// its pattern does not match is nothing to report and reaches neither of the
/// first two. A badge with a `link` is written to point somewhere, so a value
/// it cannot point at is a reference the reader has lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "undrawn", rename_all = "kebab-case")]
pub enum Undrawn {
    /// No badge at all: the pattern does not read this bead's value.
    Badge { key: String },
    /// A badge without its link: the template named a capture this value did
    /// not supply, and a destination built round a part that was never there
    /// points somewhere else.
    Link { key: String },
    /// A badge without its short form, for the same reason: a form built round
    /// a part that was never there says something the value does not. The row
    /// falls back to cutting the long form, and a narrow pane loses a badge the
    /// config meant to keep.
    ///
    /// Reported for any badge naming a `short`, `link` or no `link`: naming
    /// one is a promise about a length, made by the same config that would
    /// otherwise have declined.
    Short { key: String },
}

/// What this bead's configured badges came to: the ones it draws, and the
/// ones that fell short of what their config promised.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Badges {
    pub drawn: Vec<Badged>,
    pub undrawn: Vec<Undrawn>,
}

/// Render the configured badges that apply to this bead.
pub fn badges_for(bead: &Bead, badges: &[Badge]) -> Badges {
    let mut drawn = Vec::new();
    let mut undrawn = Vec::new();

    for badge in badges {
        let Some(value) = value_for(bead, &badge.key) else {
            continue;
        };
        // A badge with no `link` never reports: whatever it does with this
        // value, it did what its config asked.
        let promised = badge.link.is_some();

        let Some(text) = badge.apply(value) else {
            if promised {
                undrawn.push(Undrawn::Badge {
                    key: badge.key.to_string(),
                });
            }
            continue;
        };

        let link = badge.link_for(value);
        if promised && link.is_none() {
            undrawn.push(Undrawn::Link {
                key: badge.key.to_string(),
            });
        }

        let short = badge.short_for(value);
        if badge.short.is_some() && short.is_none() {
            undrawn.push(Undrawn::Short {
                key: badge.key.to_string(),
            });
        }

        drawn.push(Badged {
            key: badge.key.to_string(),
            text,
            short,
            link,
            colour: badge.colour,
        });
    }

    Badges { drawn, undrawn }
}

/// What this bead holds where the key says to read, or nothing where it holds
/// nothing there.
///
/// A bead with no external reference is a bead with nothing at that key, which
/// is the same silence as a metadata key it never wrote: the badge does not
/// draw, and there is nothing to report.
fn value_for<'b>(bead: &'b Bead, key: &BadgeKey) -> Option<&'b str> {
    match key {
        BadgeKey::ExternalRef => bead.external_ref.as_deref(),
        BadgeKey::Metadata(key) => bead.metadata.get(key).map(String::as_str),
    }
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

    fn meta(key: &str) -> BadgeKey {
        BadgeKey::Metadata(key.to_string())
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
                key: meta("blocked_on"),
                match_value: Some(matching("human")),
                render: "waiting".into(),
                link: None,
                short: None,
                colour: None,
            },
            Badge {
                key: meta("blocked_on"),
                match_value: Some(matching("dependency")),
                render: "dep".into(),
                link: None,
                short: None,
                colour: None,
            },
            Badge {
                key: meta("absent_key"),
                match_value: None,
                render: "never".into(),
                link: None,
                short: None,
                colour: None,
            },
        ];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got.drawn,
            vec![Badged {
                key: "metadata.blocked_on".to_string(),
                text: "waiting".to_string(),
                link: None,
                short: None,
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
            key: meta("xyzzy"),
            match_value: None,
            render: "→ {}".into(),
            link: None,
            short: None,
            colour: None,
        }];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got.drawn,
            vec![Badged {
                key: "metadata.xyzzy".to_string(),
                text: "→ plugh".to_string(),
                link: None,
                short: None,
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
            key: meta("jira"),
            match_value: None,
            render: "{}".into(),
            link: None,
            short: None,
            colour: Some(Colour::Status),
        }];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got.drawn,
            vec![Badged {
                key: "metadata.jira".to_string(),
                text: "ATLAS-19".to_string(),
                link: None,
                short: None,
                colour: Some(Colour::Status),
            }]
        );
    }

    #[test]
    fn a_bead_with_no_configured_badges_renders_none() {
        let bead = bead_with(r#"{"blocked_on":"human"}"#);

        assert_eq!(badges_for(&bead, &[]), Badges::default());
    }

    // ---- the bead's own external reference -------------------------------

    fn bead_referencing(external_ref: &str) -> Bead {
        let json = format!(
            r#"[{{"id":"p-1","title":"root","status":"open",
                  "external_ref":{external_ref},"metadata":{{"jira":"ATLAS-19"}}}}]"#
        );
        parse_beads(&json).expect("the bead parses").remove(0)
    }

    /// A tracker running a sync adapter holds its reference in the field
    /// rather than in metadata, and a badge reading the field draws it exactly
    /// as a badge reading metadata draws that.
    #[test]
    fn a_badge_on_the_external_reference_draws_it_as_it_draws_metadata() {
        let bead = bead_referencing(r#""https://jira.invalid/browse/HELIO-412""#);
        let cfg = vec![
            Badge {
                key: BadgeKey::ExternalRef,
                match_value: Some(matching(r".*/(?<ticket>[A-Z]+-[0-9]+)")),
                render: "{ticket}".into(),
                link: Some("https://jira.invalid/browse/{ticket}".into()),
                short: None,
                colour: None,
            },
            Badge {
                key: meta("jira"),
                match_value: None,
                render: "{}".into(),
                link: None,
                short: None,
                colour: None,
            },
        ];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got.drawn,
            vec![
                Badged {
                    key: "external_ref".to_string(),
                    text: "HELIO-412".to_string(),
                    link: Some("https://jira.invalid/browse/HELIO-412".to_string()),
                    short: None,
                    colour: None,
                },
                Badged {
                    key: "metadata.jira".to_string(),
                    text: "ATLAS-19".to_string(),
                    link: None,
                    short: None,
                    colour: None,
                },
            ]
        );
        assert_eq!(got.undrawn, Vec::new());
    }

    /// The field is empty on every bead of a tracker no sync adapter fills, so
    /// this is the case the badge meets most often. It is not a badge that fell
    /// short: it is a badge that was never about this bead.
    #[test]
    fn a_bead_holding_no_external_reference_draws_no_badge_and_reports_nothing() {
        let promising = Badge {
            key: BadgeKey::ExternalRef,
            match_value: None,
            render: "{}".into(),
            link: Some("https://jira.invalid/browse/{}".into()),
            short: Some("{}".into()),
            colour: None,
        };

        for spelling in [r#""""#, "null"] {
            let got = badges_for(&bead_referencing(spelling), std::slice::from_ref(&promising));

            assert_eq!(got.drawn, Vec::new(), "drew on {spelling}");
            assert_eq!(got.undrawn, Vec::new(), "reported on {spelling}");
        }

        let absent = bead_with(r#"{"jira":"ATLAS-19"}"#);
        let got = badges_for(&absent, &[promising]);

        assert_eq!(got.drawn, Vec::new());
        assert_eq!(got.undrawn, Vec::new());
    }

    // ---- what a badge meant to draw could not draw -----------------------

    /// The pattern a global list writes for a `delivery_pr` reads the
    /// qualified form. A tracker holding a bare number as well has beads this
    /// pattern cannot read at all, and dropping them tells the reader nothing.
    fn qualified_only() -> Badge {
        Badge {
            key: meta("delivery_pr"),
            match_value: Some(matching(
                r"(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)",
            )),
            render: "⇢ #{number}".into(),
            link: Some("https://forge.invalid/{owner}/{repo}/pull/{number}".into()),
            short: None,
            colour: None,
        }
    }

    #[test]
    fn a_badge_meant_to_draw_reports_a_value_its_pattern_cannot_read() {
        let bead = bead_with(r#"{"delivery_pr":"30"}"#);

        let got = badges_for(&bead, &[qualified_only()]);

        assert_eq!(got.drawn, Vec::new());
        assert_eq!(
            got.undrawn,
            vec![Undrawn::Badge {
                key: "metadata.delivery_pr".to_string()
            }]
        );
    }

    /// The other half of the rule, and the half the shipped waiting badge
    /// rests on: a config that names no `link` is a filter, and declining is
    /// what it was written to do.
    #[test]
    fn a_badge_configured_to_decline_stays_silent() {
        let bead = bead_with(r#"{"blocked_on":"dependency"}"#);
        let filter = Badge {
            key: meta("blocked_on"),
            match_value: Some(matching("human")),
            render: "⏸ waiting".into(),
            link: None,
            short: None,
            colour: None,
        };

        let got = badges_for(&bead, &[filter]);

        assert_eq!(got.drawn, Vec::new());
        assert_eq!(got.undrawn, Vec::new());
    }

    /// A key the bead does not carry is not a badge that fell short — it is a
    /// badge that was never about this bead.
    #[test]
    fn a_badge_whose_key_the_bead_does_not_carry_reports_nothing() {
        let bead = bead_with(r#"{"blocked_on":"human"}"#);

        let got = badges_for(&bead, &[qualified_only()]);

        assert_eq!(got.drawn, Vec::new());
        assert_eq!(got.undrawn, Vec::new());
    }

    /// The badge draws and the link does not, which is the case a reader
    /// cannot see: an ordinary-looking badge that has quietly lost its
    /// destination.
    #[test]
    fn a_badge_reports_a_link_its_value_could_not_fill() {
        let bead = bead_with(r#"{"delivery_pr":"30"}"#);
        let either_form = Badge {
            match_value: Some(matching(
                r"(?:(?<owner>[^/]+)/(?<repo>[^#]+))?#?(?<number>[0-9]+)",
            )),
            ..qualified_only()
        };

        let got = badges_for(&bead, &[either_form]);

        assert_eq!(
            got.drawn,
            vec![Badged {
                key: "metadata.delivery_pr".to_string(),
                text: "⇢ #30".to_string(),
                link: None,
                short: None,
                colour: None,
            }]
        );
        assert_eq!(
            got.undrawn,
            vec![Undrawn::Link {
                key: "metadata.delivery_pr".to_string()
            }]
        );
    }

    #[test]
    fn a_badge_that_draws_its_link_reports_nothing() {
        let bead = bead_with(r#"{"delivery_pr":"orbital/atlas#30"}"#);

        let got = badges_for(&bead, &[qualified_only()]);

        assert_eq!(
            got.drawn,
            vec![Badged {
                key: "metadata.delivery_pr".to_string(),
                text: "⇢ #30".to_string(),
                link: Some("https://forge.invalid/orbital/atlas/pull/30".to_string()),
                short: None,
                colour: None,
            }]
        );
        assert_eq!(got.undrawn, Vec::new());
    }

    /// Both forms come off one reading of the value, and both travel to the
    /// row: which of them a row can afford is the view's to decide and not
    /// this module's.
    #[test]
    fn a_badge_carries_the_short_form_its_config_named() {
        let bead = bead_with(r#"{"delivery_pr":"orbital/atlas#30"}"#);
        let both_forms = Badge {
            render: "⇢ {repo} #{number}".into(),
            short: Some("⇢ #{number}".into()),
            ..qualified_only()
        };

        let got = badges_for(&bead, &[both_forms]);

        assert_eq!(
            got.drawn,
            vec![Badged {
                key: "metadata.delivery_pr".to_string(),
                text: "⇢ atlas #30".to_string(),
                short: Some("⇢ #30".to_string()),
                link: Some("https://forge.invalid/orbital/atlas/pull/30".to_string()),
                colour: None,
            }]
        );
        assert_eq!(got.undrawn, Vec::new());
    }

    /// A badge naming no short form is a badge with one length, which is
    /// every badge written before there was a second one to name.
    #[test]
    fn a_badge_whose_config_names_no_short_form_carries_none() {
        let bead = bead_with(r#"{"delivery_pr":"orbital/atlas#30"}"#);

        let got = badges_for(&bead, &[qualified_only()]);

        assert_eq!(got.drawn[0].short, None);
        assert_eq!(got.undrawn, Vec::new());
    }

    /// The `link` rule read across to the other template: a short form built
    /// round a part that was never there says something the value does not.
    /// It is dropped and reported, so the row falls back to cutting the long
    /// form and the reader is told which key to go and look at.
    ///
    /// Reported whatever the badge's `link`, unlike the two above it. A
    /// `link` decides whether a badge *promised* to point anywhere; naming a
    /// short form is that same promise made about a length.
    #[test]
    fn a_badge_reports_a_short_form_its_value_could_not_fill() {
        let bead = bead_with(r#"{"delivery_pr":"30"}"#);
        let unlinked_either_form = Badge {
            match_value: Some(matching(
                r"(?:(?<owner>[^/]+)/(?<repo>[^#]+))?#?(?<number>[0-9]+)",
            )),
            render: "⇢ #{number}".into(),
            short: Some("⇢ {repo}".into()),
            link: None,
            ..qualified_only()
        };

        let got = badges_for(&bead, &[unlinked_either_form]);

        assert_eq!(
            got.drawn,
            vec![Badged {
                key: "metadata.delivery_pr".to_string(),
                text: "⇢ #30".to_string(),
                short: None,
                link: None,
                colour: None,
            }]
        );
        assert_eq!(
            got.undrawn,
            vec![Undrawn::Short {
                key: "metadata.delivery_pr".to_string()
            }]
        );
    }

    /// A badge that declines the value says nothing at either length, so the
    /// short form it names is nothing to report.
    #[test]
    fn a_badge_configured_to_decline_reports_no_short_form() {
        let bead = bead_with(r#"{"blocked_on":"dependency"}"#);
        let filter = Badge {
            key: meta("blocked_on"),
            match_value: Some(matching("human")),
            render: "⏸ waiting".into(),
            short: Some("⏸".into()),
            link: None,
            colour: None,
        };

        let got = badges_for(&bead, &[filter]);

        assert_eq!(got.drawn, Vec::new());
        assert_eq!(got.undrawn, Vec::new());
    }
}

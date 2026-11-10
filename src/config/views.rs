//! Views: lists of beads a setup names and defines, each opened in place of
//! the forest.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::model::types::Bead;

use super::{conditions_hold, Pattern};

/// One list of beads, under the name the reader gave it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub name: String,
    /// The beads the view lists are those any one of these selects.
    pub select: Vec<Select>,
}

/// One set of conditions a bead can meet to be in a view.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Select {
    /// The projects whose beads this selects. Every project where it names
    /// none.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Read as a badge's `when` is read.
    #[serde(default)]
    pub when: BTreeMap<String, Pattern>,
    /// Read as a badge's `unless` is read.
    #[serde(default)]
    pub unless: BTreeMap<String, Pattern>,
}

impl View {
    /// Whether this view lists `bead`, which `project`'s tracker holds.
    pub fn holds(&self, project: &str, bead: &Bead) -> bool {
        self.select.iter().any(|select| {
            (select.projects.is_empty() || select.projects.iter().any(|p| p == project))
                && conditions_hold(&select.when, &select.unless, bead)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::collect::bd::parse_beads;
    use crate::config::Config;
    use crate::model::types::Bead;

    const PROJECTS: &str = r#"
[[projects]]
name = "arkham"
path = "/home/user/arkham"

[[projects]]
name = "kadath"
path = "/home/user/kadath"
"#;

    fn configured(views: &str) -> anyhow::Result<Config> {
        Config::from_toml(&format!("{PROJECTS}{views}"))
    }

    fn bead(row: &str) -> Bead {
        let json = format!(r#"[{{"id":"p-1","title":"a bead","status":"open",{row}}}]"#);
        parse_beads(&json).expect("the bead parses").remove(0)
    }

    /// The names of the views listing `bead` in `project`.
    fn holding(cfg: &Config, project: &str, bead: &Bead) -> Vec<String> {
        cfg.views_holding(project, bead)
    }

    const TWO_VIEWS: &str = r#"
[[views]]
name = "on the desk"

[[views.select]]
when = { labels = "signed|countersigned" }

[[views.select]]
projects = ["kadath"]
when = { labels = "reminder" }
unless = { labels = "waiting" }

[[views]]
name = "urgent"

[[views.select]]
when = { priority = "0" }
"#;

    #[test]
    fn a_view_holds_a_bead_any_one_of_its_selects_selects() {
        let cfg = configured(TWO_VIEWS).expect("the config parses");

        assert_eq!(
            holding(&cfg, "arkham", &bead(r#""labels":["countersigned"]"#)),
            vec!["on the desk"]
        );
        assert_eq!(
            holding(&cfg, "kadath", &bead(r#""labels":["reminder"]"#)),
            vec!["on the desk"]
        );
        assert_eq!(
            holding(&cfg, "arkham", &bead(r#""labels":["signed"],"priority":0"#)),
            vec!["on the desk", "urgent"]
        );
        assert_eq!(
            holding(&cfg, "arkham", &bead(r#""labels":["unsigned"]"#)),
            Vec::<String>::new()
        );
    }

    /// A bead carries several labels at once, so an exclusion on one of them
    /// keeps the bead out whatever else it carries.
    #[test]
    fn an_exclusion_on_one_of_several_labels_keeps_the_bead_out() {
        let cfg = configured(TWO_VIEWS).expect("the config parses");

        assert_eq!(
            holding(&cfg, "kadath", &bead(r#""labels":["reminder","waiting"]"#)),
            Vec::<String>::new()
        );
        assert_eq!(
            holding(
                &cfg,
                "kadath",
                &bead(r#""labels":["waiting","reminder","rigging"]"#)
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_select_naming_projects_selects_only_their_beads() {
        let cfg = configured(TWO_VIEWS).expect("the config parses");

        assert_eq!(
            holding(&cfg, "arkham", &bead(r#""labels":["reminder"]"#)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_config_naming_no_views_has_none() {
        let cfg = configured("").expect("the config parses");

        assert!(cfg.views.is_empty());
        assert_eq!(
            holding(&cfg, "arkham", &bead(r#""labels":["signed"]"#)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn two_views_of_one_name_are_refused() {
        let refused = configured(
            r#"
[[views]]
name = "urgent"
[[views.select]]
when = { priority = "0" }

[[views]]
name = "urgent"
[[views.select]]
when = { priority = "1" }
"#,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            refused,
            "each view needs a name of its own, and these are used more than once: urgent"
        );
    }

    #[test]
    fn a_view_with_nothing_to_select_by_is_refused() {
        let refused = configured(
            r#"
[[views]]
name = "urgent"
select = []
"#,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            refused,
            "the view urgent has no [[views.select]], so it would list nothing; give it one"
        );
    }

    #[test]
    fn a_select_naming_a_project_nobody_configured_is_refused() {
        let refused = configured(
            r#"
[[views]]
name = "urgent"
[[views.select]]
projects = ["celephais"]
"#,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            refused,
            "the view urgent selects from celephais, but there is no project called \
             celephais; the configured projects are arkham, kadath"
        );
    }
}

//! Where a directory in a linked worktree sits in the main working tree,
//! read off what git wrote on disk with nothing run.

use std::path::{Component, Path, PathBuf};

/// The same place in the main working tree as `path`, where `path` is in a
/// linked worktree; nothing where it is not, or where the way back cannot
/// be read.
///
/// A linked worktree's `.git` is a file rather than a directory, and its
/// `gitdir:` line names the worktree's admin directory under the main
/// repository's `.git/worktrees/`. That directory's `commondir` names the
/// main repository's git directory, and the main working tree is the
/// directory over it. Each hop is a file read, so nothing runs in the
/// worktree or the repository it belongs to.
///
/// Every way the hops can fail answers nothing rather than a guess: a `.git`
/// that is a directory is a main working tree already; a file with no
/// `gitdir:` line, an admin directory that is gone, or one with no
/// `commondir` — a submodule's has none — says nothing about where the
/// worktree came from; and the main working tree is the common dir's parent
/// only when the common dir is the `.git` inside it. A bare repository and a
/// `--separate-git-dir` one both break that, and both are ordinary git: the
/// bare one has no working tree at all, and the other's is somewhere its git
/// directory's parent is not.
pub fn in_the_main_working_tree(path: &Path) -> Option<PathBuf> {
    let root = path.ancestors().find(|dir| dir.join(".git").exists())?;
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return None;
    }
    let said = std::fs::read_to_string(&dot_git).ok()?;
    let named = said
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))
        .map(str::trim)?;
    let folded = Path::new(named).is_relative();
    let admin = resolved(root, named);
    let common = std::fs::read_to_string(admin.join("commondir")).ok()?;
    let common = resolved(&admin, common.trim());
    if common.file_name()? != ".git" {
        return None;
    }
    if folded && !the_admin_directory_claims(&admin, root) {
        return None;
    }
    let within = path.strip_prefix(root).ok()?;
    Some(common.parent()?.join(within))
}

/// Whether the admin directory names `root` back as the worktree it belongs
/// to. Its `gitdir` file holds the path git wrote for the worktree's own
/// `.git`, so an admin directory arrived at by any route but this worktree's
/// says so here.
///
/// **Asked only where the `gitdir:` line was relative**, because that is the
/// only way to arrive at the wrong admin directory. `resolved` folds `..`
/// lexically, which is not what the filesystem does where the path reaches
/// the worktree through a symlink, so a relative `gitdir:` folded that way
/// can land elsewhere — usually on nothing, and the `commondir` read gives
/// up, but on a machine with two checkouts beside each other it can land on
/// a real admin directory of a repository the pane has nothing to do with.
/// Every check after that passes, because each is satisfied by any ordinary
/// repository, and the pane would be placed confidently in a stranger's
/// working tree. That is the one outcome this module refuses outright: a
/// wrong answer is worse than none.
///
/// An absolute `gitdir:` never goes through the arithmetic, so there is
/// nothing here to catch on that branch and something to lose: a worktree
/// moved without `git worktree repair` has an admin directory still naming
/// where it used to be, and asking there would refuse a placement the rest
/// of the chain derives perfectly well.
/// `a_worktree_moved_without_repair_is_still_placed` is what holds the gate
/// in place — not the symlinked rows, which an unconditional version passes
/// too, because resolving both sides makes the comparison symlink-invariant.
///
/// **The comparison resolves paths and the answer never does.** `resolved`
/// keeps `canonicalize` out because the *returned* path has to be the
/// spelling a config would write rather than git's. This returns no path at
/// all — it chooses between `Some` and `None`, and what `Some` carries is
/// still built from the lexical fold — so resolving both sides to compare
/// them costs the answer nothing. That is what keeps the guard from being
/// blunt: a worktree reached through a symlinked ancestor, where the fold
/// happens to land on the right admin directory, still matches, and so does
/// one whose `admin/gitdir` is written in the opposite form to its `.git`.
/// Strict string equality would refuse both. A path that cannot be resolved
/// is one that is not there, and answers `false`.
fn the_admin_directory_claims(admin: &Path, root: &Path) -> bool {
    let arrived_from = std::fs::canonicalize(root.join(".git")).ok();
    let claimed = std::fs::read_to_string(admin.join("gitdir"))
        .ok()
        .and_then(|named| std::fs::canonicalize(resolved(admin, named.trim())).ok());
    arrived_from.is_some() && arrived_from == claimed
}

/// `given` as a path: `base` is where a relative one starts from, and `..`
/// is folded without asking the filesystem, so a symlinked checkout compares
/// against a config the way the config wrote it.
///
/// Folding `..` lexically is not what the filesystem does, so where `base`
/// reaches the worktree through a symlink the two disagree and a relative
/// `gitdir:` can name an admin directory that is not the worktree's own.
/// That is the price of the answer being a path a config can be compared
/// against — resolving through the links would place the pane at git's
/// spelling of the checkout rather than the reader's, and a project
/// configured through a symlink would stop holding its own panes.
///
/// What the fold lands on is then a fact about the disk rather than
/// anything this decides, which is why `the_admin_directory_claims` exists:
/// usually nothing is there and the read gives up, but it can land on a
/// stranger's admin directory, and being wrong about which repository a
/// pane belongs to is worse than saying nothing. The back-check is what
/// makes the failure `None` in both cases instead of one of them.
fn resolved(base: &Path, given: &str) -> PathBuf {
    let given = Path::new(given);
    let whole = if given.is_absolute() {
        given.to_path_buf()
    } else {
        base.join(given)
    };
    let mut folded = PathBuf::new();
    for part in whole.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !folded.pop() {
                    folded.push(part);
                }
            }
            _ => folded.push(part),
        }
    }
    folded
}

/// Linked worktrees for a test to place a pane in.
#[cfg(test)]
pub mod testing {
    use std::path::{Path, PathBuf};

    /// A checkout and a linked worktree of it, each somewhere of its own.
    pub struct LinkedWorktree {
        pub scratch: PathBuf,
        pub checkout: PathBuf,
        pub linked: PathBuf,
    }

    impl Drop for LinkedWorktree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.scratch);
        }
    }

    /// An empty directory of the test's own, resolved so that what git says
    /// about it compares equal to what the test says.
    pub fn a_scratch_directory(named: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the directory is ours to make");
        std::fs::canonicalize(&path).expect("the directory we just made resolves")
    }

    /// git run with an identity and a default branch of our own, so the test
    /// says the same thing on a machine whose git is configured differently
    /// and on one where it is not configured at all.
    ///
    /// The `-c` flags settle only what this fixture needs to state; the two
    /// environment variables are what make the rest of the sentence true, by
    /// shutting out every config file git would otherwise read. Overriding
    /// four settings and inheriting the others is not machine-independence:
    /// `worktree.useRelativePaths` set globally has git write a relative
    /// `gitdir:`, which is a different fixture from the one these tests
    /// describe.
    pub fn git_in(cwd: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .args([
                "-c",
                "user.name=bdi tests",
                "-c",
                "user.email=tests@beady-eye.invalid",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A repository and a linked worktree of it, made by git itself.
    pub fn a_linked_worktree_git_made(named: &str) -> LinkedWorktree {
        let scratch = a_scratch_directory(named);
        let checkout = scratch.join("checkout");
        std::fs::create_dir_all(&checkout).expect("the directory is ours to make");
        git_in(&checkout, &["init"]);
        git_in(&checkout, &["commit", "--allow-empty", "-m", "root"]);
        let linked = scratch.join("seat/wt");
        git_in(
            &checkout,
            &["worktree", "add", "--detach", &linked.display().to_string()],
        );
        LinkedWorktree {
            scratch,
            checkout,
            linked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{a_linked_worktree_git_made, a_scratch_directory, git_in, LinkedWorktree};
    use super::*;
    use pretty_assertions::assert_eq;

    /// A linked worktree's files as git 2.55 writes them, captured from a
    /// worktree of this repository: `.git` is a file naming the admin
    /// directory outright, and the admin directory's `commondir` climbs back
    /// to the main `.git` relatively.
    fn a_linked_worktree_written_by_hand(named: &str) -> LinkedWorktree {
        let scratch = a_scratch_directory(named);
        let checkout = scratch.join("checkout");
        let linked = scratch.join("seat/wt");
        let admin = checkout.join(".git/worktrees/wt");
        std::fs::create_dir_all(&admin).expect("the directory is ours to make");
        std::fs::create_dir_all(&linked).expect("the directory is ours to make");
        std::fs::write(admin.join("commondir"), "../..\n").expect("ours to write");
        std::fs::write(
            admin.join("gitdir"),
            format!("{}\n", linked.join(".git").display()),
        )
        .expect("ours to write");
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .expect("ours to write");
        LinkedWorktree {
            scratch,
            checkout,
            linked,
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The fixture written by hand is worth only what git actually writes,
    /// so the two are compared byte for byte on the files the read follows.
    #[test]
    fn the_fixture_written_by_hand_is_what_git_writes() {
        let by_git = a_linked_worktree_git_made("worktree-by-git");
        let by_hand = a_linked_worktree_written_by_hand("worktree-by-hand");

        let relocated = |text: String| {
            text.replace(
                &by_git.scratch.display().to_string(),
                &by_hand.scratch.display().to_string(),
            )
        };
        assert_eq!(
            relocated(read(&by_git.linked.join(".git"))),
            read(&by_hand.linked.join(".git"))
        );
        assert_eq!(
            read(&by_git.checkout.join(".git/worktrees/wt/commondir")),
            read(&by_hand.checkout.join(".git/worktrees/wt/commondir"))
        );
    }

    #[test]
    fn a_directory_in_a_linked_worktree_is_placed_at_the_same_place_in_the_checkout() {
        let fixture = a_linked_worktree_written_by_hand("worktree-placed");
        let deep = fixture.linked.join("crates/dish");
        std::fs::create_dir_all(&deep).expect("the directory is ours to make");

        assert_eq!(
            in_the_main_working_tree(&deep),
            Some(fixture.checkout.join("crates/dish"))
        );
        assert_eq!(
            in_the_main_working_tree(&fixture.linked),
            Some(fixture.checkout.clone()),
            "the worktree's own root is the checkout's"
        );
    }

    #[test]
    fn a_worktree_git_made_is_placed_the_same_way() {
        let fixture = a_linked_worktree_git_made("worktree-git-placed");
        let deep = fixture.linked.join("crates/dish");
        std::fs::create_dir_all(&deep).expect("the directory is ours to make");

        assert_eq!(
            in_the_main_working_tree(&deep),
            Some(fixture.checkout.join("crates/dish"))
        );
    }

    /// The main working tree's own `.git` is a directory, and a directory
    /// under it is already where it would be placed.
    #[test]
    fn a_directory_in_the_main_working_tree_is_left_where_it_is() {
        let fixture = a_linked_worktree_written_by_hand("worktree-main");
        let inside = fixture.checkout.join("src");
        std::fs::create_dir_all(&inside).expect("the directory is ours to make");

        assert_eq!(in_the_main_working_tree(&inside), None);
    }

    #[test]
    fn a_directory_in_no_repository_is_left_where_it_is() {
        let scratch = a_scratch_directory("worktree-none");
        let loose = scratch.join("notes");
        std::fs::create_dir_all(&loose).expect("the directory is ours to make");

        assert_eq!(in_the_main_working_tree(&loose), None);

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    #[test]
    fn a_directory_that_does_not_exist_is_left_where_it_is() {
        assert_eq!(
            in_the_main_working_tree(Path::new("/srv/work/nowhere/at/all")),
            None
        );
    }

    #[test]
    fn a_dot_git_file_naming_no_gitdir_is_ignored() {
        let fixture = a_linked_worktree_written_by_hand("worktree-no-gitdir");
        std::fs::write(fixture.linked.join(".git"), "nothing git wrote\n").expect("ours to write");

        assert_eq!(in_the_main_working_tree(&fixture.linked), None);
    }

    /// The admin directory is gone — the main repository was moved or
    /// removed — and the file still names it.
    #[test]
    fn a_gitdir_that_no_longer_exists_is_ignored() {
        let fixture = a_linked_worktree_written_by_hand("worktree-gone");
        std::fs::remove_dir_all(fixture.checkout.join(".git/worktrees"))
            .expect("the directory is ours to remove");

        assert_eq!(in_the_main_working_tree(&fixture.linked), None);
    }

    /// A submodule's `.git` file has the same shape and names a directory
    /// with no `commondir`; nothing is derived from the path in its place.
    #[test]
    fn an_admin_directory_with_no_commondir_is_ignored() {
        let fixture = a_linked_worktree_written_by_hand("worktree-no-commondir");
        std::fs::remove_file(fixture.checkout.join(".git/worktrees/wt/commondir"))
            .expect("the file is ours to remove");

        assert_eq!(in_the_main_working_tree(&fixture.linked), None);
    }

    /// `commondir` may be absolute; the relative form is what git writes and
    /// what every other test here reads.
    #[test]
    fn an_absolute_commondir_is_read_as_it_is() {
        let fixture = a_linked_worktree_written_by_hand("worktree-absolute-commondir");
        std::fs::write(
            fixture.checkout.join(".git/worktrees/wt/commondir"),
            format!("{}\n", fixture.checkout.join(".git").display()),
        )
        .expect("ours to write");

        assert_eq!(
            in_the_main_working_tree(&fixture.linked),
            Some(fixture.checkout.clone())
        );
    }

    /// `worktree.useRelativePaths` has git write `gitdir:` relative to the
    /// directory holding the `.git` file.
    #[test]
    fn a_relative_gitdir_is_resolved_against_the_worktree() {
        let fixture = a_linked_worktree_written_by_hand("worktree-relative-gitdir");
        std::fs::write(
            fixture.linked.join(".git"),
            "gitdir: ../../checkout/.git/worktrees/wt\n",
        )
        .expect("ours to write");

        assert_eq!(
            in_the_main_working_tree(&fixture.linked),
            Some(fixture.checkout.clone())
        );
    }

    /// The same relative `gitdir:`, reached through a symlink. Folding `..`
    /// lexically climbs out of the link's own directory rather than out of
    /// the worktree it points at, so the admin directory is not where the
    /// fold says and the pane is left where it is.
    ///
    /// The cost is stated rather than fixed, and the two readings side by
    /// side are what state it: resolving through the link would answer git's
    /// spelling of the checkout, which is not necessarily the config's, and
    /// `a_path_in_none_of_the_working_trees_git_listed_holds_only_itself` in
    /// `discovery` is the rule that would break. An absolute `gitdir:` — the
    /// default — is placed correctly however the worktree is reached.
    #[test]
    fn a_relative_gitdir_reached_through_a_symlink_is_left_where_it_is() {
        let fixture = a_linked_worktree_written_by_hand("worktree-symlinked-relative-gitdir");
        let spelled = fixture.scratch.join("link");
        std::os::unix::fs::symlink(&fixture.linked, &spelled).expect("the link is ours to make");

        assert_eq!(
            in_the_main_working_tree(&spelled),
            Some(fixture.checkout.clone()),
            "the symlink alone is not what defeats the read: an absolute \
             gitdir is followed through it"
        );
        // That row is not what keeps the back-check from being asked on
        // every branch: resolving both sides makes it symlink-invariant, so
        // an unconditional version passes here too. What distinguishes them
        // is `a_worktree_moved_without_repair_is_still_placed` below.

        std::fs::write(
            fixture.linked.join(".git"),
            "gitdir: ../../checkout/.git/worktrees/wt\n",
        )
        .expect("ours to write");

        assert_eq!(in_the_main_working_tree(&spelled), None);
        assert_eq!(
            in_the_main_working_tree(&fixture.linked),
            Some(fixture.checkout.clone()),
            "and the relative form alone is not either: the same worktree by \
             its own path is placed"
        );
    }

    /// The same symlinked relative `gitdir:`, with a real admin directory
    /// sitting where the lexical fold lands. Nothing about the arrangement
    /// is exotic — a second checkout beside the first, and a link one level
    /// shallower than the one above — and every check but the back-check
    /// passes, because they are all satisfied by any ordinary repository.
    ///
    /// The stranger is whole rather than a stub: its worktree and the `.git`
    /// its admin directory names both exist, so what refuses the placement
    /// is the two paths naming different worktrees, and not a file that
    /// happens to be missing.
    #[test]
    fn a_folded_gitdir_landing_on_another_repositorys_admin_directory_places_nothing() {
        let fixture = a_linked_worktree_written_by_hand("worktree-folded-onto-a-stranger");
        std::fs::write(
            fixture.linked.join(".git"),
            "gitdir: ../../checkout/.git/worktrees/wt\n",
        )
        .expect("ours to write");

        let stranger = fixture.scratch.join("stranger");
        let strangers_worktree = stranger.join("seat/wt");
        let strangers_admin = stranger.join("checkout/.git/worktrees/wt");
        std::fs::create_dir_all(&strangers_admin).expect("the directory is ours to make");
        std::fs::create_dir_all(&strangers_worktree).expect("the directory is ours to make");
        std::fs::write(strangers_admin.join("commondir"), "../..\n").expect("ours to write");
        std::fs::write(
            strangers_admin.join("gitdir"),
            format!("{}\n", strangers_worktree.join(".git").display()),
        )
        .expect("ours to write");
        std::fs::write(
            strangers_worktree.join(".git"),
            format!("gitdir: {}\n", strangers_admin.display()),
        )
        .expect("ours to write");

        let spelled = stranger.join("seat/link");
        std::os::unix::fs::symlink(&fixture.linked, &spelled).expect("the link is ours to make");

        assert_eq!(
            in_the_main_working_tree(&spelled),
            None,
            "a pane in one repository's worktree was placed in another's"
        );
        assert_eq!(
            in_the_main_working_tree(&strangers_worktree),
            Some(stranger.join("checkout")),
            "the stranger is a working repository, so its own worktree places"
        );
    }

    /// The ordinary shape of a symlink: one link high up and everything
    /// under it identical. The fold pops back through the link's own
    /// spelling and lands on the right admin directory by a different name,
    /// so the placement is correct and the answer keeps the reader's
    /// spelling rather than git's.
    ///
    /// It is written down because the guard has to *not* refuse it, and the
    /// reason it does not is worth a reader's attention: `admin/gitdir` is
    /// resolved against the admin directory as we spelled it — through the
    /// link — and then resolved again to compare, so a spelling on either
    /// side cannot decide the answer. Resolving it to an absolute path "for
    /// tidiness" would look harmless and turn this row into `None`.
    #[test]
    fn a_worktree_under_a_symlinked_ancestor_is_placed_in_the_readers_own_spelling() {
        let scratch = a_scratch_directory("worktree-symlinked-ancestor");
        let real = scratch.join("real");
        let linked = real.join("seat/wt");
        let admin = real.join("checkout/.git/worktrees/wt");
        std::fs::create_dir_all(&admin).expect("the directory is ours to make");
        std::fs::create_dir_all(&linked).expect("the directory is ours to make");
        std::fs::write(admin.join("commondir"), "../..\n").expect("ours to write");
        std::fs::write(admin.join("gitdir"), "../../../../seat/wt/.git\n").expect("ours to write");
        std::fs::write(
            linked.join(".git"),
            "gitdir: ../../checkout/.git/worktrees/wt\n",
        )
        .expect("ours to write");

        let dev = scratch.join("dev");
        std::os::unix::fs::symlink(&real, &dev).expect("the link is ours to make");

        assert_eq!(
            in_the_main_working_tree(&dev.join("seat/wt/src")),
            Some(dev.join("checkout/src")),
            "the answer is the spelling the reader arrived by, which is the \
             one a config naming the link would be written in"
        );

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    /// A worktree directory moved and not repaired: its `.git` still names
    /// the admin directory, which still holds a sound `commondir`, but the
    /// admin directory's own `gitdir` still names where the worktree used to
    /// be. `git worktree repair` is what mends that, and forgetting it is
    /// ordinary.
    ///
    /// The way back is intact, so the pane is placed. This is the reading
    /// that makes the back-check's gate load-bearing rather than tidy: asked
    /// on this branch too, it would find the admin directory naming a path
    /// that is gone and refuse a placement it can perfectly well derive. An
    /// absolute `gitdir:` never goes through the fold, so there is nothing
    /// here for the back-check to catch and something for it to lose.
    #[test]
    fn a_worktree_moved_without_repair_is_still_placed() {
        let fixture = a_linked_worktree_git_made("worktree-moved-without-repair");
        let moved = fixture.scratch.join("elsewhere/wt");
        std::fs::create_dir_all(moved.parent().expect("it has a parent"))
            .expect("the directory is ours to make");
        std::fs::rename(&fixture.linked, &moved).expect("the directory is ours to move");

        let admin = fixture.checkout.join(".git/worktrees/wt");
        assert_eq!(
            read(&admin.join("gitdir")).trim(),
            format!("{}/.git", fixture.linked.display()),
            "the admin directory still names where the worktree used to be"
        );

        assert_eq!(
            in_the_main_working_tree(&moved),
            Some(fixture.checkout.clone())
        );
    }

    /// A bare repository has worktrees and no working tree of its own, so
    /// there is nowhere to place the directory.
    #[test]
    fn a_worktree_of_a_bare_repository_is_left_where_it_is() {
        let scratch = a_scratch_directory("worktree-bare");
        let bare = scratch.join("ground.git");
        let linked = scratch.join("seat/wt");
        let admin = bare.join("worktrees/wt");
        std::fs::create_dir_all(&admin).expect("the directory is ours to make");
        std::fs::create_dir_all(&linked).expect("the directory is ours to make");
        std::fs::write(admin.join("commondir"), "../..\n").expect("ours to write");
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .expect("ours to write");

        assert_eq!(in_the_main_working_tree(&linked), None);

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    /// `git init --separate-git-dir` keeps a working tree's git directory
    /// somewhere else, so the common dir's parent is a directory that is
    /// not the main working tree. Answering it would place the pane
    /// somewhere it is not, which is worse than not placing it.
    #[test]
    fn a_worktree_of_a_repository_with_a_separate_git_dir_is_left_where_it_is() {
        let scratch = a_scratch_directory("worktree-separate-git-dir");
        let checkout = scratch.join("main");
        let elsewhere = scratch.join("elsewhere.git");
        git_in(
            &scratch,
            &[
                "init",
                "--separate-git-dir",
                &elsewhere.display().to_string(),
                &checkout.display().to_string(),
            ],
        );
        git_in(&checkout, &["commit", "--allow-empty", "-m", "root"]);
        let linked = scratch.join("seat/wt");
        git_in(
            &checkout,
            &["worktree", "add", "--detach", &linked.display().to_string()],
        );

        assert!(
            std::fs::read_to_string(checkout.join(".git"))
                .expect("git wrote it")
                .starts_with("gitdir:"),
            "the main working tree's .git is a file too, and its gitdir has no commondir"
        );
        assert_eq!(in_the_main_working_tree(&linked), None);
        assert_eq!(
            in_the_main_working_tree(&checkout),
            None,
            "the main working tree is left where it is"
        );

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    #[test]
    fn a_relative_path_is_folded_without_the_filesystem() {
        assert_eq!(
            resolved(Path::new("/srv/work/main/.git/worktrees/wt"), "../.."),
            PathBuf::from("/srv/work/main/.git")
        );
        assert_eq!(
            resolved(
                Path::new("/srv/work/seat/wt"),
                "./../../main/.git/worktrees/wt"
            ),
            PathBuf::from("/srv/work/main/.git/worktrees/wt")
        );
        assert_eq!(
            resolved(Path::new("/srv/work/seat"), "/elsewhere/.git"),
            PathBuf::from("/elsewhere/.git"),
            "an absolute path owes nothing to the base"
        );
        assert_eq!(
            resolved(Path::new("./seat/wt"), "../../main/.git"),
            PathBuf::from("main/.git"),
            "a leading `.` is the only one the components leave to fold"
        );
    }
    /// A `.git` whose symlink dangles is walked past, because that is what
    /// git does with one: its repository discovery stats the name, and a
    /// link to nothing fails that stat as surely as a name nothing holds.
    /// So the directory under it is still in the worktree above, and the
    /// answer is the worktree's.
    ///
    /// **The predicate here is `stat` on purpose, and it is the opposite of
    /// the one `collect::run`'s installed probe asks with.** That one
    /// reproduces the kernel's `execvp` lookup, which stops at the entry, so
    /// it asks `lstat` and a dangling link is something installed. This one
    /// reproduces git's discovery, which resolves, so a dangling link is
    /// nothing. Reading either predicate as the crate's rule breaks the
    /// other site: `symlink_metadata` here would stop at the dangling
    /// `.git`, fail to read it as a file, and answer nothing where git
    /// answers the worktree.
    #[test]
    fn a_dot_git_whose_symlink_dangles_is_walked_past_as_git_walks_past_it() {
        let fixture = a_linked_worktree_git_made("worktree-dangling-dot-git");
        let deep = fixture.linked.join("nested/sub");
        std::fs::create_dir_all(&deep).expect("the directory is ours to make");
        let gone = fixture.scratch.join("the-target-a-build-collected");
        std::os::unix::fs::symlink(&gone, fixture.linked.join("nested/.git"))
            .expect("the link is ours to make");
        assert!(
            std::fs::symlink_metadata(&gone).is_err(),
            "the link only dangles while nothing holds {}",
            gone.display()
        );

        assert_eq!(
            in_the_main_working_tree(&deep),
            Some(fixture.checkout.join("nested/sub"))
        );
    }
}

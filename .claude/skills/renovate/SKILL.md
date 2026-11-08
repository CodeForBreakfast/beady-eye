---
name: renovate
description: Use when asked to triage, review, evaluate, drain or process beady-eye's Renovate pull requests — the minor, major and lock-file-maintenance bumps Renovate opens and holds rather than merging. Also use for a held bump of beads, the flake input that pins bd, at any update type, patch included. Triggers on "review the renovate PRs", "triage renovate", "drain the dependency queue", "the held bumps", "review the beads bump". Not for cutting a release, and not for the other patch and digest updates Renovate lands on its own.
---

# Evaluate held Renovate pull requests

`.github/renovate.json5` sets `automerge: true`, so patch and digest updates
land unattended once CI is green, behind a seven-day `minimumReleaseAge`. Three
kinds are held instead, labelled `renovate/needs-review`: **minor**, **major**,
and **lock-file maintenance**. Renovate opens them and stops.

Lock-file maintenance is held for a reason the other two do not share: it
deletes the lock and lets cargo and nix re-resolve, so the versions come from
their own resolvers and the age gate never sees them. Read it as a bulk
unaudited bump, not as a no-op.

## The contract

**One pull request per invocation, lowest risk first, then stop.** Run the skill
again for the next one.

## 1. Discover the queue

```bash
gh pr list --repo CodeForBreakfast/beady-eye --state open \
  --author "app/renovate" --label "renovate/needs-review" \
  --json number,title,mergeable,statusCheckRollup
```

## 2. Pick one, by risk

1. **minor** before **major** before **lock-file maintenance**.
2. Within a tier, prefer one whose checks are already green.

## 3. Evaluate the bump

Renovate's body carries the upstream release notes. Read it, then size the blast
radius in our own tree. What that means depends on the manager.

### github-actions

Every action here is pinned to a sha with the tag in a trailing comment, so the
bump is a one-line sha swap and the release notes are the only prose. Two checks
turn that into evidence:

**The sha is the tag it claims.** A comment is not a guarantee.

```bash
gh api repos/<owner>/<action>/git/ref/tags/<tag> --jq '.object.type + " " + .object.sha'
```

**No input we pass has gone.** Fetch `action.yml` at the old and the new sha and
diff the declared input names, then read our `with:` blocks against the result.
A removed input is the break that a release note titled "update deps" will not
mention.

```bash
gh api repos/<owner>/<action>/contents/action.yml?ref=<sha> --jq .content | base64 -d
```

Then check the **runners**. An action that drops a platform breaks only the jobs
that run there, so resolve `runs-on` for each job that uses it, including the
matrix ones in `release.yml`.

### cargo and nix

`rangeStrategy` is `update-lockfile` for cargo, which moves `Cargo.lock` alone
only while the new version still satisfies the requirement in `Cargo.toml`. A
major is outside that range by definition, so Renovate falls back to rewriting
the requirement and the manifest moves too. Read the diff for both.

Then read the changelog for behaviour changes and grep the call sites.
`nix flake check` is the evidence here, and it reads the whole tree.

### beads

The flake pins bd as its `beads` input, by tag. Review a beads bump at every
update type, a patch included, because a patch can change what bd prints and
bdi parses it.

**Read the changelog between the pinned tag and the new one.** The new tag's
`CHANGELOG.md` holds every release since the pinned one. Read each section
between the two, starting with its upgrade notes.

```bash
gh api -H 'Accept: application/vnd.github.raw' \
  'repos/gastownhall/beads/contents/CHANGELOG.md?ref=<new tag>'
```

**A release that migrates the schema is a schema decision.** The comment on
the flake's `beads` input says why the pin and the tracker's schema move
together. Name the migration in a comment, leave the pull request open, and
stop.

**Check every entry against what bdi asks of bd.** `src/collect/` spells every
bd command line bdi runs, and this lists them:

```bash
grep -n 'asked(&\[\|written(&\[\|run("bd"' src/collect/*.rs
```

An entry matters to bdi when it does any of these:

- It renames, removes or changes a subcommand or flag one of those lines uses,
  or one that `PASSED_THROUGH` lets `bdi bd` pass on.
- It changes a field in the JSON that bdi parses beside each call.
- It changes the text of an error that `src/collect/run.rs` classifies, such as
  an unknown flag.
- It changes a Dolt table that bdi's two `sql` statements read.

**A green check says little about the new bd.** Most tests serve captured bd
output through a shim rather than run bd. For each entry that matters, run the
command line in the bump's dev shell against a scratch tracker, and compare
what it prints with what bdi parses.

**Look for what the release lets bdi drop or simplify.** A fix upstream can
retire a workaround, and a new flag or field can replace a read bdi assembles
by hand. A fallback for an older bd is different. bdi supports every bd back
to the floor README states under *What it needs*, whatever the flake pins. So
the fallback for a bd without `ready --brief` goes only once that floor rises
past the bd it serves, and raising the floor is its own user-facing change.

**Fix a break in the bump's own pull request.** Push the fix to Renovate's
branch. Renovate then stops rebasing that branch, so merge `main` into it
yourself when `main` moves. File each opportunity as its own bead, as step 5
says.

## 4. Gate

**CI on the head commit is the gate, and for a workflow-only change it is the
only one.** `nix flake check` never reads `.github/`, so running it locally
against a sha bump proves nothing.

For a change the flake can see, the local gate has to run on the bump's own
tree rather than on yours. Check the head out first, and name its sha —
`read-ci-verdict` with no argument answers for whatever `HEAD` is, which on
your own branch is a verdict about something else entirely.

```bash
gh pr checkout <number>
check-before-push
read-ci-verdict "$(git rev-parse HEAD)"    # until it answers
```

## 5. Stop rather than force

- **Extensive breaking changes.** Comment what the upgrade would need, leave the
  pull request open, and stop. A held major is a fine outcome.
- **A peer or ecosystem constraint.** Not ours to force. Name the blocker in a
  comment and leave it open for Renovate to keep rebasing. Don't pin or
  downgrade something else to make it pass.
- **An opportunity the new version opens** — a workaround we can now delete, a
  simpler API — goes in its own bead, never in the bump. Key the bead on the
  package name: Renovate reopens the same update under a new number.

## 6. Merge

There is no branch of our own to push. Check the branch is up to date, then
squash-merge the pull request with the merge command your setup provides,
deleting its branch, and confirm the result.

```bash
gh api repos/CodeForBreakfast/beady-eye/compare/main...<head sha> --jq .behind_by
# squash-merge <number> here, deleting the branch
gh pr view <number> --json state,mergeCommit   # the exit code is not the check
```

**Renovate wrote the title, and the title is the squash subject.** Run
`conventional-subject '<title>'` before merging. A Renovate title that fails it
is a `commitMessageTopic` fix in `.github/renovate.json5`, not a retitle.

**Two held bumps touching one file: the second is stale the moment the first
lands.** Renovate rebases on its own schedule. Wait for the rebase or leave the
second for the next invocation, and re-read its checks either way — a green tick
predating the merge was earned against a base that no longer exists.

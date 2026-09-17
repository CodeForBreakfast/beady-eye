---
name: renovate
description: Use when asked to triage, review, evaluate, drain or process beady-eye's Renovate pull requests — the minor, major and lock-file-maintenance bumps Renovate opens and holds rather than merging. Triggers on "review the renovate PRs", "triage renovate", "drain the dependency queue", "the held bumps". Not for cutting a release, and not for the patch and digest updates Renovate lands on its own.
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

`rangeStrategy` is `update-lockfile` for cargo, so the manifest does not move and
`Cargo.lock` does. Read the changelog for behaviour changes, then grep the call
sites. `nix flake check` is the evidence here, and it reads the whole tree.

## 4. Gate

**CI on the head commit is the gate, and for a workflow-only change it is the
only one.** `nix flake check` never reads `.github/`, so running it locally
against a sha bump proves nothing.

For a change the flake can see, gate it the way `CLAUDE.md` says:
`check-before-push`, then `read-ci-verdict` until it answers.

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

Delivery is `CLAUDE.local.md`'s, with no branch of our own to push.

```bash
gh api repos/CodeForBreakfast/beady-eye/compare/main...<head sha> --jq .behind_by
gh pr merge <number> --squash --delete-branch
gh pr view <number> --json state,mergeCommit   # the exit code is not the check
```

**Renovate wrote the title, and the title is the squash subject.** Run
`conventional-subject '<title>'` before merging. A Renovate title that fails it
is a `commitMessageTopic` fix in `.github/renovate.json5`, not a retitle.

**Two held bumps touching one file: the second is stale the moment the first
lands.** Renovate rebases on its own schedule. Wait for the rebase or leave the
second for the next invocation, and re-read its checks either way — a green tick
predating the merge was earned against a base that no longer exists.

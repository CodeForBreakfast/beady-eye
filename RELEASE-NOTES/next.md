bdi 0.25.0

Minor release, **0.24.0 → 0.25.0**. `bdi gates` hears more about a pull request, so if you run it with `--listen`, read the first highlight.

## Highlights

**The webhook listener takes reviews, comments, check suites and statuses.** Besides `pull_request`, `bdi gates --listen` now settles a pull request on a `pull_request_review`, an `issue_comment` on a pull request, a `check_suite` or a `status` delivery. A check suite or status names a commit, so it settles the open pull requests whose head is that commit. Once you are on this release, add the *Pull request reviews*, *Issue comments*, *Check suites* and *Statuses* events to your webhook. An older `bdi gates` answers those deliveries `202` and does nothing with them. Without them, the timed look still finds everything, only later.

**A gh:pr gate can settle when its pull request is approved.** Give the gate `awaits=approved` in its metadata and it closes once GitHub's review decision is approved, or once the pull request merges. A repository that requires no reviews has no review decision, so there the gate waits for the merge.

**`bdi gates` tells a held-back bead what is happening on its pull request.** While a pull request is open, draft or not, `bdi gates` comments on each bead its gh:pr gates hold back when its checks fail, when its head conflicts with its base, when someone submits a review, and when someone comments on its conversation. Each is told once: a failure or conflict once per head commit, a review or comment once each. The gates stay open. A comment names the reviewer or author, and links the review or comment. Only what happened after the gate was made is told, so upgrading does not replay a pull request's history to the beads already waiting on it. Where your token cannot read what one of these needs, `bdi gates` skips only that kind of news, says so once for each repository, and still settles the gate.

## Maintenance

Webhook deliveries are handled one event type at a time, and gates settle through one table of pull request events.

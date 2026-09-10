# Release notes

One file per release, `RELEASE-NOTES/<version>.md`, where `<version>` is the
`MAJOR.MINOR.PATCH` the release bumps to — so `RELEASE-NOTES/0.3.0.md`. The
pull request that bumps the version checks it in.

The [`Release` workflow](../.github/workflows/release.yml) cuts the GitHub
Release from the file it finds. What is written here is what a reader gets,
word for word. A version bump merged without its notes file fails the run, so
landing the file afterwards is the whole of the repair.

## Style

The audience is somebody running `bdi`, and nobody else. Group by what a reader
would notice rather than by the change that delivered it.

**Length follows impact.** A change a reader has to act on earns a short
paragraph. A change they will simply notice earns a sentence. Everything else
earns a share of one line. Most releases are shorter than this file.

- **Highlights** — the changes somebody would notice, about the behaviour
  rather than how it was built. Say what a reader has to do about it. Where the
  answer is nothing, say that.
- **Maintenance** — one line for the whole of it, however many commits it
  covers. Dependency bumps, CI, tests, refactors and docs go here unsplit,
  named only where a reader would otherwise be surprised.
- Leave out what a reader cannot act on or notice: why a fix works, what the
  code used to do, which check now enforces it, which bead asked for it.

**One line per paragraph and per bullet.** GitHub renders a Release body with
hard line breaks on, so every newline inside a paragraph reaches a reader as a
`<br>` and a wrapped file shows as ragged short lines. A fenced block keeps its
own breaks. `nix flake check` refuses a wrapped `RELEASE-NOTES/<version>.md`.
This file is a repository document rather than a Release body, so it stays
wrapped.

## Template

```markdown
bdi <version>

<Major|Minor|Patch> release, **<previous> → <version>**.

## Highlights

**<Headline change>.** What it changes for somebody using bdi, in a sentence or
two.

## Maintenance

Dependency bumps and internal improvements.
```

# Release notes

One file per release, `RELEASE-NOTES/<version>.md`, where `<version>` is the
`MAJOR.MINOR.PATCH` the release bumps to — so `RELEASE-NOTES/0.3.0.md`. The
pull request that bumps the version checks it in.

The [`Release` workflow](../.github/workflows/release.yml) cuts the GitHub
Release from the file it finds. What is written here is what a reader gets,
word for word. A version bump merged without its notes file fails the run, so
landing the file afterwards is the whole of the repair.

## Style

User-facing and impact-classified: what changed for somebody running `bdi`.
Group by what a reader would notice, not by the change that delivered it.

- **Highlights** — the changes somebody would notice, a short paragraph each,
  about the behaviour rather than how it was built.
- **Maintenance** — dependency bumps and internal changes worth recording and
  not worth headlining.
- Say what a reader has to do about it: a config change, a migration. Where the
  answer is nothing, say that.

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

**<Headline change>.** What it changes for somebody using bdi, and why that
matters.

## Maintenance

- <a dependency, the lockfile, an internal change>
```

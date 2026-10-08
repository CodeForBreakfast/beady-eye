# Release notes

The notes for the coming release live on `main`, in `RELEASE-NOTES/next.md`,
written in the template below. A pull request that changes what a reader would
notice updates the file as it lands, and the file is brought up to date before
an rc is cut. Every rc publishes it as its GitHub Release body.

The file's first line names the version the coming release bumps to, and an rc
reads its version from that line. When a patch grows into a minor, edit that
line and the one under it. Nothing is renamed or thrown away. beady-eye stays
below 1.0 until it is settled, so the version moves by minor or patch only.

The real release's pull request bumps the version and renames the file to
`RELEASE-NOTES/<version>.md`, where `<version>` is the `MAJOR.MINOR.PATCH` it
bumps to, so `RELEASE-NOTES/0.3.0.md`. It leaves a fresh `next.md` behind,
holding the template's first two lines for the patch after it.

The [`Release` workflow](../.github/workflows/release.yml) cuts the GitHub
Release from `<version>.md`. What is written here is what a reader gets, word
for word. A version bump merged without its notes file fails the run, so
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
own breaks. `nix flake check` refuses a wrapped notes file, `next.md` included.
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

## The Claude Code plugin

The plugin releases on a version of its own, the one
`plugin/.claude-plugin/plugin.json` declares. So its coming release's notes
live in `RELEASE-NOTES/plugin/next.md`, its release renames them to
`RELEASE-NOTES/plugin/<version>.md`, and that release is tagged
`plugin-v<version>`. The audience is somebody running the plugin in Claude
Code. Everything above holds, with `beady-eye plugin <version>` as the first
line.

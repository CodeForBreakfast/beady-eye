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

The audience is somebody running `bdi`, and nobody else. They have never opened
this repository, never read a pull request and never seen a bead. Group by what
they would notice rather than by the change that delivered it.

**Run `writing-for-humans` first, then `misplaced-emphasis`.** The first fixes
pitch: what the reader needs, in what order, in words they know. The second only
cuts, so on a badly pitched draft it makes a shorter draft that is still
unreadable.

**Hold every sentence against one test.** Would somebody who has never seen this
repository know what they would see on their screen, or what they have to do?
If a sentence needs the code to be understood, rewrite it as what the reader
sees. If it cannot be put that way, the reader cannot notice the change, and it
goes in Maintenance or out.

**Repository vocabulary is the usual failure.** A writer who knows the
implementation reaches for its words without noticing. Do not use a name from
the code, a module, a type, an internal state, a config field the reader does
not set, a bead id, a pull request number, or a word you coined while working
on the change. If a word is not in the README, in `bdi --help`, or on the
screen, the reader does not have it. Name the key they press, the line they
read, the file they edit.

**Length.** A release stays under about a page. A change a reader has to act on
earns a short paragraph of at most three sentences. A change they will simply
notice earns one or two. Everything else shares one line. Most releases are
shorter than this file. A long release is long because it has many changes, so
cut each of them rather than dropping the small ones.

- **Highlights** — the changes somebody would notice, in the order of how much
  they matter, each led by a bold headline that says what happens, not what was
  built. Say what a reader has to do about it. Where the answer is nothing, say
  that.
- **Upgrading** — only when a reader has to do something or something they rely
  on now behaves differently. Leave the section out otherwise.
- **Maintenance** — one line for the whole of it, however many commits it
  covers. Dependency bumps, CI, tests, refactors and docs go here unsplit,
  named only where a reader would otherwise be surprised.
- Leave out what a reader cannot act on or notice: why a fix works, what the
  code used to do, which check now enforces it, which bead asked for it.

**No installation steps.** A release body does not say how to install `bdi`.
The README carries that, which is one edit instead of one per version, and a
reader on a Release page is already where the binaries are.

**The files already published are not the model.** The versioned files in this
directory were written without a test or an example, and several fail it. Do not match the file beside yours. Hold your draft against this
section.

**One line per paragraph and per bullet.** GitHub renders a Release body with
hard line breaks on, so every newline inside a paragraph reaches a reader as a
`<br>` and a wrapped file shows as ragged short lines. A fenced block keeps its
own breaks. `nix flake check` refuses a wrapped notes file, `next.md` included.
This file is a repository document rather than a Release body, so it stays
wrapped.

### Before and after

The same change from 0.8.0, said the way the old notes said it and the way this
guide asks for it.

Before:

> **A bead's id is now shortened against the bead it is drawn under.** Most ids
> get shorter. One gets longer: a bead drawn under one it blocks rather than
> under its parent now draws its whole id, instead of reading as that bead's
> child.

After:

> **Ids are shorter.** bdi leaves out the start of a bead's id when it matches
> the bead above it. A bead shown under one it blocks, rather than under its
> parent, now shows its whole id, so it no longer looks like a child.

The first uses "drawn under" for two different things and never says what a
reader would see. The second says what is on the screen.

## Template

```markdown
bdi <version>

<Major|Minor|Patch> release, **<previous> → <version>**.

## Highlights

**<Headline change>.** What it changes for somebody using bdi, in a sentence or
two.

## Upgrading

**<What to check or change>.** Only when a reader has to do something.

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

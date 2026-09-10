# Contributing to beady-eye

Thanks for looking. [README.md](README.md) says what `bdi` is for; this says how
to work on it.

## Reporting a bug, or proposing a change

Open a [GitHub issue](https://github.com/CodeForBreakfast/beady-eye/issues).

The maintainers track their own work in a beads tracker that is not part of this
repository and that you do not need. `.beads/` is gitignored and nothing here
names that tracker; GitHub issues are the channel for everyone else.

A security bug is the exception. Those go through [SECURITY.md](SECURITY.md),
because an issue publishes the flaw before there is a fix.

## Getting a toolchain

The flake supplies all of it:

```console
$ nix develop
```

That gives you `cargo`, `rustc`, `rustfmt`, `clippy` and `rust-analyzer`, plus
the `mutation-test-this-change`, `check-before-push` and `conventional-subject`
commands. With [direnv](https://direnv.net) installed, `direnv allow` puts the
same shell on every `cd` into the tree.

Inside it, `cargo build` and `cargo test` work as usual.

## Tests

Unit tests live in `#[cfg(test)]` modules inside the file they cover. `tests/`
holds the integration tests, which either link the library or drive the built
binary through a pty. Fixtures under `tests/fixtures/` are captures of what `bd`
and `herdr` put on the wire.

**Only invented examples go in this repository.** Every example it carries — a
fixture, inline test data, a sample config, a worked example in the docs — is
made up. Never a real capture: no real pane id, working directory, session name,
host, bead description, client or project. A real value that looks harmless is
still a real value.

That needs saying because capturing is how this project tests, and a capture
brings whatever was on the wire that day along with it. So invent the ground
first and capture against that. `tests/fixtures/` and the docs' worked examples
share one invented vocabulary; extend it rather than starting a second.

[CLAUDE.md](CLAUDE.md) has the working notes on the traps: what a pty test must
do about draining the terminal, and how an absence assertion goes wrong on this
screen. Read it before writing a test of either kind.

## Before you push

```console
$ check-before-push
```

`nix flake check` is every check CI makes against the tree: the build and the
tests, `clippy -D warnings`, `cargo fmt --check`, a `cargo package` verify, and
the repository's own tree scans. `check-before-push` runs it, and refuses a dirty
tree first. That refusal matters — the check reads the git index, so an
uncommitted file is not in the source it checks, and a green result would not
have compiled it.

A check in the flake's `checks` output is a check CI runs, and nothing runs that
is not there. Adding one there is how you add one.

**Green here is not all of CI.** One job never reads the tree: `conventional
subject` reads your pull request's title, and a title is nowhere in the source a
flake check can see. The next section is that job's rule.

## Opening a pull request

Everything reaches `main` through a pull request, squash-merged.

**The title becomes the commit subject.** It is the only line of your branch that
`main` keeps, and CI refuses one that is not a conventional commit:

    type(scope): description

- **type** is one of `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`,
  `refactor`, `revert`, `style`, `test`. A `!` before the colon marks a breaking
  change, and [README.md](README.md#status) says what that does to the version.
- **scope** is optional, and closed: `collect`, `app`, `model`, `view`, `tui` —
  the five layers under `src/` — plus `ci`, `flake`, `docs`, `tests`, `deps`.
  Leave it out rather than coin one; a new scope is a change to the check.
- **description** is lower case, in the imperative, and has no full stop. It has
  to finish the sentence *"If applied, this commit will …"*, so `draw a bead id
  in its status colour` rather than `draws`, `drew` or `drawing`. An acronym or a
  name keeps its capitals.
- **72 characters**, counting the whole title.

Get the same verdict CI will give, before you open anything:

```console
$ conventional-subject 'fix(view): draw a bead id in its status colour'
```

**The body becomes the commit body.** Say what changed and why; the diff already
says how the code does it.

## Where things are

`src/` is five layers:

- `collect/` runs `bd` and `herdr` and parses what they say.
- `app/` decides what each tracker is asked for and keeps what came back.
- `model/` joins the two into a snapshot: the tree, its badges, and the anomalies
  where the two sources disagree.
- `view/` turns a snapshot into rows and draws them.
- `tui/` runs the loop and owns the terminal.

`docs/design.md` is the spec — a starting point rather than gospel, deviated from
as the project learns. `docs/plans/` holds the implementation plans written
against it.

Three rules the project was designed under, and a change should keep:

- **Terminology comes from beads or herdr.** Don't invent a word where either
  project has the concept. Where both are silent, coin one and add it to the
  terminology table in `docs/design.md` marked *coined*.
- **No coupling to any agent workflow.** `bdi` knows nothing about how agents are
  organised. A convention a setup encodes in bead metadata is named in config and
  drawn without interpretation; anything that needs to know what a metadata key
  *means* belongs in config rather than in the model.
- **Degrade, never disappear.** An unreachable tracker, a filtered tree, a
  dangling parent — each is reported rather than silently dropped.

## Licence

`beady-eye` is [Apache 2.0](LICENSE), and under section 5 of that licence a
contribution you submit is under the same terms unless you say otherwise.

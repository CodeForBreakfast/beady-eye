# beady-eye

`bdi` is one unblinking eye over every [beads](https://github.com/gastownhall/beads)
tracker you point it at. It draws each tracker's work as a tree, and beside
every bead an agent has claimed, the live [herdr](https://herdr.dev) pane that
agent is sitting in.

beads knows the work. herdr knows the agents. Neither has heard of the other,
so neither can tell you that `atlas-5` was claimed by a pane that died on
Tuesday. Something old has opened an eye over both, and it can.

![A bdi screen: the atlas project over twelve beads in two trees, each bead drawn with its status glyph and its id in that status's colour, three of them with a green agent marker and pane id beside them, one warning that a bead is claimed with no pane behind it, three panes below that no bead claims, and a band at the foot showing what is on the selected bead's pane.](docs/bdi-frame.svg)

The atlas project is invented. `atlas-5` is the drift: claimed, with nothing
behind the claim.

The eye only looks. It never writes to a tracker. Changing the work is still
`bd`'s job, and the eye finds this arrangement acceptable.

## Summoning

Homebrew fetches a built binary, for macOS and Linux on Intel and arm64:

```console
$ brew install codeforbreakfast/tap/bdi
```

crates.io, if you would rather compile it yourself:

```console
$ cargo install beady-eye
```

Nix, with flakes on, builds the tip of `main`. Nothing caches it, so the first
run is a cup of tea:

```console
$ nix run github:CodeForBreakfast/beady-eye
```

From your own flake, pin a release tag and take the package or the overlay:

```nix
inputs.beady-eye.url = "github:CodeForBreakfast/beady-eye/v0.3.0";

beady-eye.packages.${system}.default                # the package
nixpkgs.overlays = [ beady-eye.overlays.default ];  # pkgs.beady-eye
```

Or take a binary from the [latest release](https://github.com/CodeForBreakfast/beady-eye/releases/latest).
There is one per platform with a `.sha256` beside it, and the Linux ones are
static. Rename it `bdi` and put it on your `PATH`. Apple has not been asked to
sign it, so a copy a browser downloaded needs
`xattr -d com.apple.quarantine bdi` before macOS will let it open its eye.

The crate is `beady-eye`. The command is `bdi`.

## Opening the eye

Stand in a repository beads tracks and run it:

```console
$ bdi
```

No config. It reads that project, draws its trees, and keeps them fresh. Three
bands: the forest at the top, the tail of the selected bead's pane under it,
and a foot row with notices on the left and keys on the right.

Keys to start with. `?` lists the lot.

| key | does |
|---|---|
| `↑` `↓` `j` `k` | move |
| `←` `→` `h` `l` | collapse or expand; again to move to the parent or first child |
| `Enter` | open the bead, and from there, focus its pane |
| `f` | focus the selected bead's pane |
| `a` | every tree, not only the ones with a live agent |
| `/` `n` `N` | search ids and titles |
| `y` | copy the bead id (OSC 52, so it survives ssh and a multiplexer) |
| `^R` | read the trackers again now |
| `q` | avert the eye |

`bdi --json` writes the same snapshot to stdout instead of drawing it. That
is also what to reach for when stdout is not a terminal; `bdi | cat` says so and
exits.

## Several trackers

A config file opens the eye on all of them:

```toml
[[projects]]
name = "atlas"
path = "/home/you/atlas"

[[projects]]
name = "orbital"
path = "/srv/work/orbital"
environment_command = "nix develop -c"
```

That lives at `~/.config/beady-eye/config.toml`, or wherever `--config` says,
and an edit takes effect while `bdi` runs. Start it under one of the projects
and it reads that one alone. Start it anywhere else, or pass `--all-projects`,
and it reads them all. `--project NAME` picks.

Each project is read with its own `bd`, entered the way you would enter it
yourself. An `.envrc` and direnv need nothing said. Anything else, say it with
`environment_command`.

[docs/configuration.md](docs/configuration.md) has the rest: badges drawn from
bead metadata, credentials, extra roots, intervals, the light theme, and the
socket you can poke to say a tracker changed so the eye stops polling it.

## What it needs

- **Linux or macOS.** Unix sockets and unix signals. Windows would need a
  different ritual entirely.
- **bd 1.1.0 or newer.** An older one is reported on the project's line rather
  than obeyed. Packaged builds lag, so check what yours says.
- **A tracker bd can open.** Server or embedded. A Dolt server wants a
  password, and it reaches bd in `BEADS_DOLT_PASSWORD`, from your shell or from
  a project's `credential_command`.
- **herdr, for the agents.** Without it you get the trees and the claims, and
  every tree is drawn. With it you get the point: which claim has a live pane
  behind it, which pane toils on nothing any bead has heard of, and the tail.
- **git, for worktrees.** Without it the project is named after its directory
  and a pane cannot be placed by worktree.
- **A terminal that honours OSC 52**, for `y`. Terminal.app does not, and says
  nothing about it.

## Status

Released, and gazed into daily by the people who wrote it. The design is in
[docs/design.md](docs/design.md).

Versions are `0.x`, and a breaking change bumps the minor. Pin a tag. `1.0.0`
arrives when the shape has settled, not when something breaks.

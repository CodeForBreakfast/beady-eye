# bdi's visual language

`docs/design.md` is the spec, and says what each surface draws. This file asks
the question the spec never puts to itself: **is the same treatment the same
statement wherever the reader meets it?**

A reader learns a terminal program the way they learn any notation — by
noticing that something means something, and then finding it means the same
thing next time. `bdi` draws five surfaces, and each acquired its colours in
its own bead from its own argument. This is the first reading that puts them
side by side.

It is a review and a recommendation, and the beads it spawns are where the
work happens. Several of them have landed — answer 2's list is
`src/view/palette.rs` — and the tracker rather than this file is what says
which.

Everything below was read at `22acc0d`, over `src/` excluding
`src/view/sgr.rs`, which replays the colours a pane wrote and chooses none of
its own. Every `<file>:<line>` here is at that commit; `git show
22acc0d:<file>` is what they name.

Three sections read outside the codebase and say so where they start: what
ratatui and crossterm emit, read from their source; what terminals do with the
intensity attribute; and what other programs do about a theme they cannot see.
Those last two are a survey done for this bead on 2026-09-03 rather than
measured here. **Sources** at the end carries the URLs and file paths for
every claim in them, and lists what the survey's own author flagged as
inferred, which is excluded from the argument.

## If you are here for one thing

This is long because it is a survey. Seven pieces of work fall out of it, and
each of those seats needs one section rather than the argument.

| you are | read |
|---|---|
| taking `bdi-kbd2`, the liveness scale | *3. The property a tier relationship must hold* — the arithmetic, the four routes, and **ground and two rungs, not three** |
| taking `bdi-7lie`, the tail's voice | *`NO_COLOR` removes colour and keeps attributes*, then *What the intensity attribute actually does on a terminal* — dim and italic each fail somewhere |
| building the one palette | *2. One palette, in one place* — the list is the deliverable |
| deciding what a channel may carry | *1. The channel assignment* — six channels, one kind of fact each |
| wondering whether `bdi` should detect the theme | *4. Whether `bdi` follows the reader's theme* — the answer is no, and what to do instead |
| reconciling `docs/design.md` (`bdi-2bb.1`) | *What this changes in `docs/design.md`* — four extensions and one reframing |
| checking a claim | *Sources*, at the end |

**The three findings that would change what you build**, if you read nothing
else:

1. **There is no portable third rung, on any channel.** Palette slots, RGB and
   the intensity attribute each fail on a real reader's setup, and all three
   fail on the *background* rather than on the theme. So the scale is a ground
   and two rungs.
2. **Redundancy is a floor on the harm, not a detector of the fault.** Every
   meaning already has a glyph, which is why the last two defects were
   survivable — and why a redundancy gate would have been green through both.
   It does not substitute for sizing the intervals.
3. **`bdi` should not ask the terminal what theme it is.** A one-way OSC
   degrades to nothing; a query degrades to a wait or to a lie, and the cheap
   negative fails inside a multiplexer, which is where `bdi` lives.

## The table

Every meaning `bdi` draws, down the side; every surface that draws it, across
the top.

Tokens: `default` is `Color::Reset` — the terminal's own foreground; `grey8`
is `Color::DarkGray`, which reaches the terminal as its palette slot 8;
`grey↓` is the fixed `Color::Rgb(108, 118, 128)` that `tone.rs` calls `DIM`;
`bd-*` are `bd`'s own 24-bit literals. `plain` is a span that names no style
and takes whatever the row it sits on carries. `—` means the surface does not
draw that meaning at all.

| meaning | forest: bead row | forest: project line | forest: group, item, note | bead window: head | bead window: page | tail band | bindings window | status bar |
|---|---|---|---|---|---|---|---|---|
| **bead status** | glyph in `bd-*` | — | — | glyph in `bd-*`, status word in `bd-*` | related row's glyph in `bd-*` | — | — | — |
| **bead identity** | plain | — | — | `bd-blue` | — | — | — | — |
| **a live agent** | `green` | `green` | `green` | `green` | — | — | — | — |
| **wants looking at** | `yellow` | `yellow` | `yellow` | — | — | — | — | `yellow` |
| **how live this row is** | `default` / `grey8` / `grey↓` | off the scale — all `default` | — | — | — | — | — | — |
| **finished** | `grey↓` | — | — | — | closed related row `grey↓` | — | — | — |
| **the cursor is here** | `REVERSED` | `REVERSED` | `REVERSED` | — | — | — | — | — |
| **tree structure** | `default` | `default` | fold arrow `default` | — | — | — | — | — |
| **a landmark** | — | — | — | heading `default` + `BOLD` | — | — | window title `BOLD` | — |
| **an edge** | — | — | — | arrow `default` | — | — | — | — |
| **nothing went wrong here** | — | — | `default` | — | — | — | — | — |
| **an affordance** | — | — | `grey8` | window title `BOLD` | — | — | rows `plain`, title `BOLD` | keys `plain` |
| **metadata / age** | — | mark and age `grey8` | — | facts line `plain` | — | — | — | — |
| **chrome** | — | — | — | border `plain` | — | rule `grey8` | border `plain` | — |
| **an explanation** | note `yellow` | unread reason `yellow` | item `yellow` | — | — | reason `grey8` | — | notice `yellow` |
| **the body of a page** | — | — | — | — | `grey8` | — | — | — |
| **prose: emphasis** | — | — | — | — | `ITALIC` | — | — | — |
| **prose: strong** | — | — | — | — | `BOLD` | — | — | — |
| **prose: heading** | — | — | — | — | `BOLD` | — | — | — |
| **prose: code** | — | — | — | — | `cyan` | — | — | — |
| **prose: link** | — | — | — | — | `UNDERLINED` | — | — | — |
| **prose: quote** | — | — | — | — | `│` bar, `plain` | — | — | — |
| **the pane's own words** | — | — | — | — | — | replayed verbatim | — | — |

## What the table shows

Five findings and the rule the sixth left behind once it was answered, then
two things that look like findings and are not. Each is
confirmed against the sites rather than argued from the shape of the code.

### One grey says five unrelated things

`Color::DarkGray` has nine production sites and they mean four different
things, plus a fifth that is load-bearing on its own:

| site | means |
|---|---|
| `tone.rs:40` (`UNSTAFFED`) | *nobody is working on this* |
| `tone.rs:48` (`PAGE`, aliased to `UNSTAFFED`) | *this is the body of a page* |
| `groups.rs:36`, `groups.rs:63` | *here is a key you can press* |
| `project.rs:75`, `project.rs:89` | *this is metadata, glance past it* |
| `tail.rs:89` | *this is a rule, not content* |
| `tail.rs:55`, `tail.rs:65` | *this is `bdi` explaining itself* |

The last of those is the one that cannot simply be recoloured. `design.md`
says of the tail: *"What is dimmed is what `bdi` says in the band, which is
the whole of what tells its words from the pane's."* So in the tail band the
grey is a **voice** marker — the only channel separating `bdi`'s sentences
from the pane's — while eight rows higher, on the same screen at the same
instant, it says *nobody is working on this*.

Three of the nine reach the value through a name (`UNSTAFFED`, `PAGE`), and
six spell `Color::DarkGray` inline. There is therefore no single place where
changing this grey changes it.

### `Color::Reset` says both *most important* and *not content at all*

This is the sharper of the two, and it is not in the bead. `Color::Reset` has
six production sites across three names and three literals:

| site | means |
|---|---|
| `tone.rs:34` (`STAFFED`) | **the top of the liveness scale** — an agent is on this row |
| `show.rs:29` (`STANDS_OUT`) | emphasis — a heading, an edge arrow |
| `mod.rs:253` (`structure`) | box-drawing: *this is not content* |
| `groups.rs:31`, `mod.rs:219` | *nothing went wrong here* |

In the forest, the terminal's default foreground is the row you came for. In
the bead window it is a heading against a dimmed page. On the box-drawing it
is the thing you read past. On a group line it is the absence of a warning.
The same value is the strongest statement on one surface and the weakest on
the next.

It also does a second job that is not a meaning at all. Because a `Fitted`
draws `Line::from(spans).style(self.whole)` and a span's own style patches
over the line's, `fg(Color::Reset)` is how a span **escapes the row's tone**.
`structure()` uses it for exactly that. So `Reset` is simultaneously a value
on the liveness scale and the mechanism for opting out of it.

That collision is visible on screen and `tone.rs` already documents it, in the
helper `the_words`:

> *a staffed row's box-drawing shares its style and merges into it while an
> unworked row's stands apart.*

Which is to say: **on the one row the reader came for, the tree-drawing and
the text are indistinguishable.** The tier that earns the screen is the tier
that loses the structure.

### The bottom of the liveness scale is a status colour

`DIM` is `Color::Rgb(108, 118, 128)`. It is the finished tier on the
brightness scale, and it is `bd`'s own hue for the `deferred` **status**.
`tone.rs` says so deliberately — *"`bd` draws a deferred bead's glyph and
every cell of a finished row in this one grey, so one name serves both."*

The intent is sound: match `bd`. The consequence is that the two channels are
not independent at that point, and the existing gate cannot see it.
`no_colour_is_given_to_two_statuses_and_only_open_goes_without_one` walks the
status set and asserts no two share a value. It is scoped to one channel, so a
value shared *across* channels — a status hue that is also a brightness rung —
passes it silently.

### Brightness carries liveness and nothing else

In the forest, brightness is liveness: default, then slot 8, then the fixed
grey. The bead window spends none of it — its head and its page are both the
terminal's own foreground — so the reader who learns *dim means nobody is on
it* meets nothing on the next surface that says otherwise. What stands out on
a page of text takes a weight, which the body of a page leaves unspent.

The window covering the forest is not what makes that safe, and the argument
that it is turns up whenever a second meaning is proposed for a channel. The
two surfaces are never on screen together; the reader is on both of them, and
carries what a treatment meant on the first to the second.

The channels with exactly one client each — `REVERSED` for the selection,
`ITALIC` for emphasis, `UNDERLINED` for a link — have never produced a defect,
and this is what that buys.

### One meaning is drawn at both ends of the emphasis range

*Here is a key you can press* is drawn two ways, and they are as far apart as
the palette goes:

| where | said | drawn |
|---|---|---|
| forest, hidden-trees group | *a to show all* | `grey8` — the quietest thing on the row |
| forest, scoped line | *--all-projects reads every project* | `grey8` |
| bead window, border title | *`<id>` · Esc to go back* | `BOLD` — the loudest thing in the window |
| bindings window, border title | *Key bindings · press any key to close* | `BOLD` |

There is a defence, and it is in `design.md` — a window's title is the way out
of a modal view, and *"a reader who cannot see how to leave is stuck in a view
they may have opened by accident"*, where a filter toggle is optional and can
afford to recede.

That defence is about **urgency**, and the scheme has no channel for urgency.
So what the table exposes here is not a mistake but a missing axis: two facts
of the same kind, correctly given different weight, using two treatments that
mean other things elsewhere — `grey8` also meaning *nobody is on this* and
`BOLD` also meaning *this is a landmark*.

It is the sharpest question the table raises that this review does not settle.
Either affordances get one treatment and the modal escape earns its prominence
some other way — position, which it already has, being the only thing on the
border — or urgency becomes a declared axis with its own channel and its own
rule. Both are defensible; picking one is work this review hands on rather than
does.

### The scale mixes theme-relative and absolute values, so its intervals are nobody's property

The three tiers are `Color::Reset`, `Color::DarkGray` and
`Color::Rgb(108, 118, 128)`. The first two are resolved by the reader's
terminal at draw time; the third is fixed. So:

- **staffed → unstaffed** is one theme value against another. It moves with
  the theme, and the process holds neither number.
- **unstaffed → finished** is a theme value against a constant. It moves with
  the theme *relative to a fixed point*, which is the interval that can
  collapse without anything else changing.

Both defects to date are instances of this, and they failed by two different
mechanisms:

- **`bdi-sw4`** painted the staffed tier `Color::White` — palette slot 15 —
  against a theme whose `color15` and `foreground` are the same hex. The two
  tiers were one. The mechanism is that **the default foreground is not a
  palette slot; it is an alias for whatever the theme chose**, and it is
  routinely the same value as slot 7 or 15.
- **`bdi-kbd2`** is the second interval — slot 8 against the fixed grey — at
  1.46:1 on both of the palettes tabled under answer 3, which is what makes it
  a property of the scale rather than of a theme.

`tone.rs`'s status test works because `bd`'s literals are absolute — the
process holds all four numbers, so distinctness is a computation. Nothing
equivalent exists for the tiers, and nothing can while the values live in the
terminal rather than in `bdi`.

That is the structural finding, and it is why a seventh point fix would not
help. Both defects reached Graeme rather than a gate, and the reason is the
same both times: **`bdi` does not hold the values it is comparing.**

### Two things the table turns up that are not defects

A table that only produced findings would be suspect, so these are recorded
too, with the reason each survives.

**A bead's id is drawn two ways and both are right.** In the forest it is
`Span::raw` and takes the row's own tone; in the bead window's head it is
`bd`'s blue. That reads as an inconsistency and is not one: each surface is
quoting a different `bd` command, and `show.rs:22-23` says so — *"`bd show`'s
own colour for the id at the head of the page, read off `bd` 1.2.2's
output"*. The forest leaves it plain for a stated reason too, so the row's
brightness reaches it. Two quotations of two sources, not one meaning drawn two
ways.

**The key-bindings window uses none of the palette at all.** Its rows are
plain, its border is plain, and its only treatment is `BOLD` on the title.
That is the most consistent surface `bdi` has, and it is consistent because it
draws almost nothing — which is worth saying plainly rather than counting as a
success. It sets the floor the others should be measured against: a surface
earns a treatment by having a distinction to make.

## What the tools actually do

Three facts about ratatui and crossterm decide what any recommendation is
allowed to say. All three were read from source at ratatui `main` /
`ratatui-crossterm` 0.1.2 over crossterm 0.29, and the colour mapping and
modifier diff are byte-identical to ratatui 0.29.

**The sixteen named `Color`s are palette slots, not the classic ANSI codes.**
ratatui maps `Color::DarkGray` to crossterm's `DarkGrey`, and crossterm's
`Display for Colored` writes `5;8` — so what reaches the terminal is
`\e[38;5;8m`, and slots 0–15 are the terminal's own configurable palette. So
`DarkGray` really does follow the reader's theme, and `Color::White` really is
slot 15. That is the mechanism behind `bdi-sw4` exactly: slot 15 against a
theme whose `color15` equals its `foreground`.

**ratatui's own docs say otherwise and are wrong for this backend.**
`color.rs` documents `DarkGray` as *"Bright Black. Foreground: 90"*. The
crossterm backend emits no `90` anywhere. The doc describes the concept; the
source describes the wire. Design against the source.

**`Color::Rgb` is emitted with no capability check at all.** Nothing on the
path consults `COLORTERM` or `TERM`; ratatui's own docs say the display is
"unpredictable" on a terminal without truecolor. So `bd`'s four status
literals and the id's blue are a hard truecolor dependency that `bdi` never
states.

**All nine `Modifier` bits are emitted; none is dropped.** Which matters for
one of them in particular: **`Modifier::DIM` is entirely unspent by `bdi`'s own
drawing.** The type is already in the codebase — `sgr.rs:107` folds SGR 2 into
it when replaying a pane — so the intensity attribute is a channel `bdi` knows
how to represent and has never chosen. `BOLD`, `DIM` and their shared reset
(SGR 22) are three states of *one* attribute, which is a different kind of
thing from three palette slots: a theme can alias two slots to one value and
routinely does, and it has no way to alias bold to normal.

### `NO_COLOR` removes colour and keeps attributes — and that costs `bdi` something

crossterm 0.29 honours [no-color.org](https://no-color.org/): with `NO_COLOR`
set, `SetColors` writes nothing at all. `SetAttribute` has **no such guard**,
so bold, dim, reverse and italic still reach the terminal. `bdi` mentions
`NO_COLOR` nowhere in `src/`, so this is untested behaviour rather than a
decision.

Worked through the table above, most of it survives, and that is
`design.md:1504-1505`'s rule earning its keep — *"the glyph says the status and
the words say the rest, so a terminal with no colour loses nothing"*:

- statuses keep their glyphs; the live agent keeps `◍`; a warning keeps `⚠`;
- the selection is `REVERSED`, an attribute, so it survives;
- the three liveness tiers collapse to one, but a staffed row still carries
  `◍` and a finished one still carries `✓`, so the reader can still tell them
  apart from the row's own content.

**One thing does not survive.** In the tail band, `design.md:1570` makes the
grey the *only* channel separating `bdi`'s sentences from the pane's. Under
`NO_COLOR` that grey is gone, and so are the pane's own colours — both go
through the same suppressed call — leaving `bdi`'s words and the pane's
identically plain. The one place the spec says colour is load-bearing on its
own is the one place `NO_COLOR` breaks.

That is not an argument for ignoring `NO_COLOR`. It is the argument for
`VOICE` being a channel of its own in answer 2: if what separates `bdi` from
the pane is an *attribute* rather than a colour — the band is `bdi`'s
indentation already, and `DIM` or italic would survive — the rule holds
everywhere, including here.

### What ratatui offers for themes: nothing, on purpose

`ratatui_core::style::palette` is two modules, `tailwind` and `material`, and
both are hard-coded `Color::Rgb` design-system ramps. Their own module docs say
they include black and white *"to avoid being affected by any terminal theme
that might be in use"* — which is the opposite of what `bdi` wants. There is no
`Theme` type, no semantic naming, and no terminal query anywhere in the
library.

The ecosystem convention is a hand-rolled struct, and ratatui's own `demo2`
example is the reference shape: one `const Theme`, fields named by **role**
rather than by colour (`tabs_selected`, `description_title`, `key_binding`),
and — the detail worth taking — the fields are **`Style`, not `Color`**, so a
slot can carry a modifier as well as a hue. That is what lets a rung be
"slot 8 plus `DIM`" rather than needing a distinct slot of its own, and it is
why the palette in answer 2 should be a list of `Style`s.

For learning the reader's actual theme, `terminal-colorsaurus` is the current
maintained answer: OSC 10/11 for foreground and background, a timeout, a
heuristic that decides whether the terminal will answer *before* sending
anything, and no escapes at all under `TERM=dumb`. `termbg` is the older,
narrower one. `ratatui-themes` exists but ships its own fixed palettes, so it
has `tailwind`'s trade-off with better names.

**One hazard**, reported by the reading rather than measured here: an OSC query
writes to the tty and reads the reply back off it, so if it runs after
crossterm's event reader is live the reply can be swallowed by the event
stream. The safe placement is before raw mode is entered. Anything answer 4
recommends has to sit there.

### What the intensity attribute actually does on a terminal

Answer 3 weighs putting a rung of the scale on `Modifier::BOLD` or
`Modifier::DIM` instead of on a colour, so what those two emit has to be
established before the routes are compared rather than after. Both are read
from a survey done for this bead, cited below; neither was measured here.

**Bold is a weight change on `bdi`, not a brightness change.** Many terminals
do brighten on SGR 1 — xterm maps colours 0–7 to 8–15 with `boldColors`
defaulting to *true*, wezterm does both with `bold_brightens_ansi_colors`
defaulting to true, and Windows Terminal's `intenseTextStyle` defaults to
`"bright"` and is *only* bright. VTE flipped `bold-is-bright` off in 0.56,
Alacritty's `draw_bold_text_with_bright_colors` defaults false, and kitty
refuses the behaviour outright: *"That escape code changes the FONT FACE from
regular to heavy weight. It DOES NOT affect colors."*

**But the brightening is an indexed palette remap, so it cannot apply to the
terminal's default foreground.** With `Color::Reset` there is no index to map.
wezterm documents exactly this — *"this brightening effect does not apply when
text uses the default foreground color"* — xterm's `boldColors` is defined as
the 0–7 → 8–15 mapping and so is inapplicable by construction, and Alacritty's
`compute_fg_rgb` takes the bright path only for a named palette colour, with
`NamedColor::Foreground` falling through unless the optional
`bright_foreground` is set, which is `None` by default. Alacritty and wezterm
each expose an opt-in that *does* brighten the default foreground; both are off
by default.

That is the one useful property in this whole space: **bold applied to the
default foreground is a weight step that no theme and no background can
collapse.** Every colour treatment `bdi` has is a step whose size is a property
of the reader's theme; this one is a property of the reader's font.

**Dim is a different matter, and it is the finding that changes answer 3.**
SGR 2 renders nearly everywhere in 2026 — xterm, VTE, konsole, kitty, Ghostty,
Alacritty, iTerm2, wezterm, Windows Terminal since July 2020, xterm.js and so
VS Code, Apple Terminal — with urxvt the notable drop. tmux passes it through:
`dim` is a plain terminfo capability in both `screen-256color` and
`tmux-256color`, and unlike `strikethrough` it is not gated behind
`terminal-features`. So *"DIM is a usual casualty"* is dated as a claim about
rendering.

**What is not portable is what it means.** Three mechanisms are in the field:

| terminal | mechanism | background-aware |
|---|---|---|
| xterm, VTE | multiply each foreground component by ⅔ | no |
| Alacritty | a separate dim palette entry (`DIM_FACTOR = 0.66`) | no |
| Windows Terminal | blend toward black | no |
| kitty | `dim_opacity`, default 0.4 — alpha toward the background | yes |
| Ghostty | `faint-opacity`, default 0.5 | yes |
| iTerm2, xterm.js | shift the colour toward the background | yes |

**The first group inverts on a light theme.** A light theme's default
foreground is near black, and ⅔ of near black is nearer black — so faint comes
out *darker than normal text against a white ground*, which is more prominent
rather than less. Windows Terminal's own issue #16493 measured it: *"The `2m
Faint` text is always the darkest of the three in each line… which against a
White background makes it effectively 'intense/bold' instead of 'faint'"*, and
the same report concludes *"apps cannot use `2m` (Faint) if they want to behave
reasonably when a Light theme is active."*

**So git ships the composition and the composition has the bug.**
`GIT_COLOR_FAINT_DEFAULT "\033[2;39m"` is faint over the default foreground —
the exact form a `bdi` scale on this channel would use — and on xterm, VTE,
Alacritty or Windows Terminal with a light theme that is the inverted case. The
prior art is real and it is not a clearance.

One further composition hazard, sourced rather than measured: dim combined with
reverse fails on wezterm, GNOME Terminal and Foot while working on Alacritty
and Windows Terminal. `bdi` draws the selection `REVERSED`, so a dim rung would
meet that combination on the selected row specifically.

**None of this is about `bdi` as it stands**, and the distinction matters for
reading the table above: `bdi`'s bottom rung is `Color::Rgb(108, 118, 128)`, an
absolute grey. `bdi` emits SGR 2 nowhere. The survey bears on a route the
project might take, not on a defect it has.


### What other programs do about a theme they cannot see

`bdi` is not the first program to want three distinguishable tiers against a
palette it does not hold. The distribution across widely-installed terminal
programs is lopsided, and it is lopsided in a direction that matters here.

| strategy | programs |
|---|---|
| named ANSI-16 only | git, ripgrep, GNU `ls`/dircolors, lazygit, gitui, eza, htop |
| fixed truecolor or 256 themes | btop, k9s, lsd, fzf |
| runtime theme detection | delta, bat, neovim, helix (and vim, via `COLORFGBG`) |
| a theme file with a shipped default | everything in the second row, plus helix, bat, gitui |

The first row dominates by count and by ubiquity — it is what the old,
pipe-oriented and universally-installed tools do. ripgrep states the intent
outright in `crates/printer/src/color.rs`: *"the color choices are meant to be
fairly conservative that work across terminal themes"*, and its whole default
is four named colours and one bold. The third row is entirely post-2024 and
clusters around one library: `terminal-colorsaurus` drove both the delta and
the bat integrations.

**On the question this document is actually asking — does anyone build a
three-tier intensity scale against an unknown theme — the answer is yes, and
git is the instance.** `git range-diff`'s dual colouring is faint / normal /
bold on one axis while colour carries add-versus-remove on the other, which is
exactly the separation of channels answer 1 arrives at. `color.h` defines
`GIT_COLOR_NORMAL ""`, `GIT_COLOR_BOLD "\033[1m"`, `GIT_COLOR_FAINT "\033[2m"`
and a full `GIT_COLOR_FAINT_*` family — **including `GIT_COLOR_FAINT_DEFAULT
"\033[2;39m"`, faint composed with the default foreground**, which is the
composition a `bdi` scale on this channel would need, spelled out by a project
that ships to every terminal there is. `--color-moved=dimmed-zebra` is a second
instance, and it stacks two attribute channels at once
(`GIT_COLOR_FAINT_ITALIC "\033[2;3m"`).

**helix is the second instance, and it went there for `bdi`'s reason.** Its
`term16_*` themes have no spare colours, so `dim` carries `comment`, `hint`,
all three inlay-hint scopes, `namespace` and `constant`. It also ships
`modifiers = ["bold", "dim"]` on `constant`, which wezterm's maintainer calls
*"self-contradictory since it's asking the terminal emulator to simultaneously
increase and decrease the intensity"* — a caution about composing the two ends
of this attribute rather than about using it.

**And there is a real countervailing set, which is why this is evidence rather
than a precedent to follow.** lazygit's entire decoration vocabulary is `bold,
underline, reverse, strikethrough` — it *cannot* emit dim. gitui never uses
ratatui's `Modifier::DIM`. btop defines `Fx::d = "\x1b[2m"` and never calls it.
lsd has no attribute keys at all, GNU `ls` ships no `02` anywhere, and
Neovim's own TUI only gained SGR 2 rendering in 0.12. So two programs do this
deliberately, several decline it deliberately, and **the ones that decline are
the ones with the widest cross-terminal exposure.**

Two findings cut against a palette-slot answer and are worth having on the
record before answer 3 chooses:

**lazygit removed its light theme rather than keep guessing.** v0.36.0's notes
read *"use better colour defaults (note: theme.lightTheme is no longer a
thing)"*, and what replaced it was `[default]` for the large surfaces and
`[bold]` where a colour would have gone. A mature TUI reached the problem this
document is about and moved *off* the palette and onto an attribute.

**eza made the same trade in the opposite direction.** Its issue #1406 reported
temp files invisible on light themes because they were drawn in an explicit
`White`; the fix replaced the hue with `Style::default().dimmed()`. That swaps
*a colour that fails on one background* for *an attribute that may not render
at all* — which is the better failure is a judgement, not a free win, and it is
the judgement answer 3 has to make.

One inversion is worth avoiding by name. **fzf picks its default theme by
colour depth rather than by background**, in `src/tui/light_unix.go`: if `TERM`
contains `256`, or `tput colors` exceeds 16, it returns `Dark256`. A reader on
a *light* Ghostty gets the dark theme precisely because their terminal is
capable; `Light256` exists and is reachable only through an explicit
`--color=light`.

**The pattern running through every program that survives on both
backgrounds: paint accents, never surfaces.** git's `CONTEXT` is the empty
string. fzf's `Fg` and `Bg` are `defaultColor` in all three of its themes.
dircolors comments out `NORMAL` and `FILE`. lazygit's two largest surfaces are
`[default]`, gitui uses `Color::Reset` for focused chrome, and htop passes
ncurses `-1` for the background in every scheme but one. `bdi` already obeys
this — it sets no background anywhere outside the selection's `REVERSED` — and
it is worth writing down as a rule it is keeping rather than a fact about the
current code.

**Provenance.** This section is a reading of those projects' sources and issue
trackers, done for this bead and spot-checked in two places against the files
quoted. Three of its claims were flagged by the reading itself as inferred
rather than sourced, and are excluded above: git's rendering of hunk headers on
light backgrounds (sourced from blogs, and git has no issue tracker), htop's
rationale for its "Broken Gray" scheme, and the *absence* of background
detection in git, fzf, gitui, lazygit, k9s, btop, eza and lsd — established by
reading their colour and theme code rather than by exhaustive search, so a
strong negative rather than a proof.


## Four answers

### 1. The channel assignment

One kind of fact per channel, and the channel says which kind before the value
says which one.

| channel | carries | rule |
|---|---|---|
| **hue** | *what kind of thing this is* — a bead's status, and the two facts `bd` cannot say: a live agent, and something wrong | quoted from `bd` where `bd` has an opinion; `bdi`'s own two are the only additions |
| **brightness** | *how live this row is* — and nothing else, on any surface | the one axis `bdi` adds to `bd list`; every other use of brightness gives it up |
| **weight** | *this is the thing to go to* — a row with an agent on it, a section name, a heading, the way out on a window's border | one reading per surface, and on the forest the top of the liveness scale as well |
| **reverse** | *the cursor is here* | exactly one client, and it stays that way |
| **italic** | emphasis, inside rendered prose | never outside `markdown.rs` |
| **underline** | *somewhere to go* — a reference in prose, and a badge whose config gave it a `link` | one meaning wherever it is drawn |

`design.md:1506-1512` already argues most of this, from the other end: it says
a row has *"exactly two colour-carrying channels — the glyph carries the
status, matching `bd`, and the row's own text carries how live it is"*, and
declines to draw priority because a third would have to share one of them.
This extends that reasoning from the bead row to the whole screen.

**Weight is the one channel spent on both surfaces**, and what makes that
sound is that it is one meaning rather than two: a row with an agent on it is
where the reader is heading, and so is the name of the section they are
looking for on a page of text. That is the test a second meaning has to pass,
and *the two surfaces are never on screen together* is not it.

It costs something and the cost is worth writing down. A staffed forest row
and an unworked one are both at the terminal's own foreground, so the weight
on the staffed one is the whole of what separates them: on that surface
weight is the top rung of the liveness scale as well as a landmark, spent
there because the scale has run out of brightness at the ground. Inside the
window it makes no further distinction — a `bd show` section name, a heading
in the prose and a bold word in a sentence are one treatment, and what tells
them apart is position, the first two owning their row and the third sitting
inside one.

Three consequences:

**Brightness carries no hierarchy in the bead window.** Its head and its page
are both the terminal's default, and what stands out on the page takes a
weight. The window spends no brightness at all, and *dim* means one thing on
every surface.

**`Color::Reset` stops being a rung.** See answer 3 — it is the ground the
scale is measured against, not a value on it.

**Prose keeps a namespace of its own.** `markdown.rs`'s cyan for code and its
italic are inside rendered text, where the reader is reading an author's words
rather than scanning `bdi`'s. They do not have to fit the scheme above, and
they must not leak out of it: nothing outside `markdown.rs` draws italic or
that cyan.

**The underline is the exception, because it says the same thing on both
surfaces.** A reference in prose and a badge carrying a `link` are both
somewhere the reader can go, so one treatment covers them and the namespace
rule has nothing to separate. It is also the whole of what says so: a
destination is nowhere in what a row draws.

### 2. One palette, in one place

Three sets exist — `tone.rs`'s consts, `show.rs`'s two, `markdown.rs`'s one —
and six sites reach past all three to spell `Color::DarkGray` inline. So the
first requirement is not *which values* but *that there is one list*.

Named for what they mean, not what they are:

| name | today | meaning |
|---|---|---|
| `STATUS_*` | `bd`'s four 24-bit literals — `open` has none, on purpose | quoted from `bd`; absolute on purpose |
| `IDENTITY` | `Rgb(89, 194, 255)` | `bd`'s own for an id; same reason |
| `AGENT` | `Green` | a live agent is here |
| `ATTENTION` | `Yellow` | this wants looking at |
| `TIER_STAFFED` / `TIER_OPEN` / `TIER_FINISHED` | `Reset` / `DarkGray` / `Rgb(108,118,128)` | the liveness scale — see answer 3, all three move |
| `STRUCTURE` | `Reset` | box-drawing and fold arrows: not content |
| `QUIET` | `DarkGray` | metadata, chrome, an affordance, a rule |
| `VOICE` | `DarkGray` | in the tail band only: these are `bdi`'s words, not the pane's |
| `CODE` | `Cyan` | prose namespace |

`QUIET` and `VOICE` are the same value today and are listed apart because they
are different claims. `design.md:1570` makes `VOICE` load-bearing — it is the
only thing separating `bdi`'s sentences from the pane's — so it is the one that
must not be quietly folded into the others when the greys are next touched.

**Each slot is a `Style`, not a `Color`.** That is ratatui's own convention
— `demo2`'s `Theme` is a single `const` of `Style` fields named by role —
and it is what lets a rung be *the default foreground plus a weight* rather
than needing a distinct slot of its own — the composition answer 3 finds is
the only one no theme can collapse. A palette of `Color`s cannot express
half the treatments in the table above; a palette of `Style`s expresses all
of them, `REVERSED` and the weights included.

The list is the deliverable of this answer. That it is one list, that every
site goes through it, and that no site spells a `Color::` or a `Modifier::` of
its own, is the requirement.

It lives in `src/view/palette.rs`, and the `palette` flake check is what holds
every other module to it — test code aside, where a literal is what pins a
value rather than choosing one. The list carries three slots the table above
does not: `PLAIN`, the terminal's default where nothing went wrong, and `HEAD`
and `PAGE`, the bead window's two tones. Each holds another slot's value and
is a claim of its own, so `bdi-1xf9` can move the window without moving the
scale and `bdi-kbd2` the scale without moving the window.

### 3. The property a tier relationship must hold, and how it could be tested

This is the answer that stops the next collision, and it is the one a point
fix cannot supply.

**The property, in three parts — and a fourth this section has to add,
because the third turns out to be unreachable.** A *scale* is an ordered set of
treatments saying how much of one quantity something has. `bdi` has one:
liveness, three rungs.

- **Ordered.** The rungs run in the order of the quantity. Testable today over
  the enum.
- **Exclusive.** No rung is also a value on another channel, and no value
  outside the scale sits between two rungs. Testable today, and it is the part
  that would catch `DIM` doubling as the `deferred` status hue.
- **Separated.** Adjacent rungs are perceptibly different on the reader's
  actual terminal. **Not testable today** — and the routes below find it is
  not reachable at all for three rungs, which is what forces the fourth part.

**Why the third is not testable, precisely.** `Color::Reset` and
`Color::DarkGray` are deferred references: the process holds a symbol and the
terminal holds the value. A test can compare symbols, and the defect lives in
the values. `tone.rs`'s status test works only because `bd`'s literals are
absolute — the process holds all four numbers, so distinctness is a
computation rather than a hope.

**The check itself is four lines.** Given both numbers, WCAG 2 relative
luminance settles it. Two independent readings exist, over two different
palettes, and they are given apart because their provenance differs:

| interval | palette as `bdi-sw4` recorded it | live theme file today |
|---|---|---|
| staffed (default fg) → unstaffed (slot 8) | 2.454 | 2.442 |
| unstaffed (slot 8) → finished (`DIM`) | 1.461 | 1.460 |
| `bdi-sw4` as shipped: `White` → unstaffed, where `color15 == foreground` | 1.000 | — |

The left column is this document's arithmetic over values read out of
`bdi-sw4`'s description — `foreground #ede0da`, `color8 #a08d83` — against
`Rgb(108, 118, 128)` from `tone.rs:25`. The right column was computed by
another seat from `themes/noctalia.conf` as it stands today, which reads
`foreground #e4e1e4`, `color8 #909098`: the theme was rewritten from warm to
neutral at some point after `bdi-sw4` recorded it. That file is outside this
repository and those figures are reported rather than re-taken here.

**Two palettes, two readings, agreement to two decimal places, and neither
interval clears 3:1 on either.** That is a stronger claim than either reading
alone supports: the finding is a property of the scale rather than of one
theme.

It is also a warning about how the left column was obtained. A palette value
quoted from a bead's description carries no ref and no date, so nothing about
reading it can tell you it has since gone false — and here it had. The
agreement is luck. **Where an argument turns on a colour, read it from the
file**; this is the same defect one level up from the one the document is
about, and for the same reason: a value nobody holds is a value nobody can
check.

**Row two was not a known defect when this review started, and is one now.**
The shipped top interval — the pair `bdi-sw4` was fixed to create — is 2.44:1
on the theme it was tuned against, below the 3:1 usually taken as the floor
for a non-text distinction. So neither interval of the current scale clears
3:1. `bdi-kbd2`'s description now carries all three rows and the conclusion,
handed to that bead rather than filed beside it. Whatever `bdi-kbd2` does
should be sized against both rungs, or it will land the same collision one rung
up.

**And that row is the worked example for this whole section.** `bdi-sw4` was a
real defect, correctly diagnosed, measured on Graeme's own terminal, decided by
Graeme from a table of candidates, implemented, and covered by a `painted()`
test that still passes. Every step was done well. The pair it created is 2.44:1
and nobody noticed, because **no threshold had ever been chosen** — the 63.1%
in that bead is kitty's glyph-renderer coverage at his cell size, which is a
different quantity on a different scale, and nothing in the process put the
result against a floor.

That is the gap this section exists to close, and it is not a gap in care. A
number was measured and a decision was taken from it; there was simply nothing
for the number to be *compared against*. Choosing the floor is therefore as
much of the work as building the check.

So the property is not hard to state and the check is not hard to compute.
**The only thing between the project and the gate is that two of the three
inputs live in the reader's terminal instead of in `bdi`.**

**The move: make the theme an input.** Today the theme is applied *downstream*
of `bdi`, by the terminal, so `bdi` never holds it. If instead a `Theme` — the
sixteen slots, the default foreground, the default background — is a value the
view resolves its semantic names against, then:

- separation becomes a computation over that value;
- the test is a table over a corpus of real themes, asserting the property for
  each, and it exists **whether or not `bdi` can ever query a live terminal**;
- Graeme's noctalia goes in that corpus as a regression, because it is the one
  theme known to have broken this twice.

That last point is what makes this a gate rather than a hope. The corpus test
needs no terminal, no query, and no runtime detection. It needs only that the
palette be produced by a function of a theme rather than baked as symbols the
terminal will interpret later.

**Four ways to get there, and every one of them bleeds somewhere.**

**(a) Make the scale absolute.** All three tiers `Color::Rgb`. Separation
becomes trivially computable. It is also `bdi-sw4` with the sign reversed — a
fixed scale is right on the theme it was tuned for and wrong on the rest — and
`design.md:1496-1498` already rejected it in favour of colour 8. Rejected.

**(b) Make the scale purely relative, and test a surrogate.** Every rung a
palette slot; none of them `Color::Reset`. The test asserts three distinct
slots and no default. This cannot promise separation, only non-identity — but
note which defects it catches: `bdi-sw4` **yes**, because the default
foreground stops being a rung; `bdi-kbd2` **yes**, because the fixed grey stops
being one. Both known defects, for a small change and a test that runs today.

**Its blind spot is bigger than it looks, though, and this document had it
wrong.** The obvious reading is that (b) only fails on a theme that sets two
slots close together, which is a misconfigured theme. But a scale of slots has
to put *something* at the top, and the only slots conventionally near the top
of a theme's brightness range are 7 and 15 — which are near-white in almost
every theme, light ones included, because a theme's slot 15 answers *"what is
bright white"* and not *"what stands out against my background"*. So (b)'s top
rung disappears on a light theme. That is `bdi-sw4` again with the background
reversed rather than the sign, and it is not a misconfiguration: it is half the
field.

**(c) Put a rung on the intensity attribute.** Bold on the default foreground
is the one treatment in this whole space whose step size no theme and no
background can change — the brightening that would confound it cannot apply to
a colour with no palette index, and what is left is a font-weight step. Nothing
else `bdi` can reach has that property.

**Three things stood against it when this was written, and the first no longer
does.** Answer 1's weight rule read *"structural only, never a degree of
anything"*, and a liveness rung is a degree, so (c) was ruled out by the
channel assignment before any measurement was taken. The assignment above no
longer says that: the forest does spend a weight on a staffed row, and the
rule beside the channel names the cost rather than forbidding the spend. What
stands against (c) is the two measurements below, which are about how big a
step it is rather than about which channel may carry what.

**The second is a measurement.** `bdi-sw4`
drove kitty's own glyph renderer at Graeme's 7×16px cell and put bold at
**+14.9%** ink against the finished tier's **−76.8%** — about a fifth of the
step that works — and the decision recorded on that bead is explicit:
*"Not bold (a fifth of the tier that works) and not two tiers."* The chosen
fix, dropping the open row to `color8`, measured 63.1%. Bold is a small step
and it was rejected knowingly.

**But the two numbers are not the same kind of quantity, and nothing put them
on separate axes.** 63.1% is a step that can go to zero when the theme changes,
and on this project's own theme it already has, twice. 14.9% is a step that
cannot move with the theme at all — only with the font, and only if the font
has no real Bold face, which is what makes it Regular-to-SemiBold here rather
than Regular-to-Bold. A small step with a floor and a large step with none are
different offers, and the table they were compared on measured only ink.

**The third is that the bottom of this channel is not available.** SGR 2
renders nearly everywhere now, but on xterm, VTE, Alacritty and Windows
Terminal it multiplies the foreground toward black rather than toward the
background, so on a light theme faint comes out *more* prominent than normal —
and `bdi` draws the selection `REVERSED`, which is the combination that fails
on wezterm, GNOME Terminal and Foot. So this channel offers a safe rung above
the ground and no safe rung below it.

**(d) Hold the theme.** As above. Separation becomes a real computation, the
corpus test becomes possible, and where `bdi` can learn the reader's actual
theme it can check the property at runtime and say so rather than draw two
identical rungs in silence.

### Nothing above gives three portable rungs, and that is the answer

Not one of (a), (b) or (c) survives contact with both a light and a dark
reader, and (d) buys it only by ceasing to be portable — it works by holding
the theme rather than by being safe without it. Two rungs are reachable on any
channel. **Three are not, and no amount of care in choosing the channel makes
them so**, because every mechanism available resolves against a value `bdi`
does not hold.

So the third part of the property — *Separated* — cannot be satisfied in
general, and a document that stopped at the routes would be recommending one of
three ways to lose. **The way out is that `bdi` never needed it.** Every
meaning on this scale is already carried by something else on the same row: a
staffed row carries `◍`, a finished one carries `✓`, and both are glyphs in the
row's own text. The scale is reinforcement. It has been read as the carrier
because it is the part that keeps breaking.

That gives the property its fourth part:

- **Redundant.** No meaning rests on a scale interval alone. For every meaning
  the scale expresses, some channel that is not a colour expresses it too.

**Redundancy is testable today, on the harness that already exists.**
`Painted`'s `Run` holds the whole `Style` beside the words it was drawn with,
so a test can assert of every liveness state that the row's *text*
distinguishes it — no terminal, no theme, no query, and no contrast
arithmetic. It is the only part of the property whose test needs nothing `bdi`
does not have.

#### What redundancy does not do, which the first draft of this section got wrong

**It would not have caught either known defect, and this document said it
would.** The claim is false in the simplest possible way: `◍` and `✓` were
already on those rows the whole time. A redundancy gate would have been
**green** through `bdi-sw4` and green through `bdi-kbd2`, because in both of
them the meaning was intact and only the tone had collapsed. Two readers
objected to that sentence independently — an adversarial review with none of
this context, and the orchestrator — and they were both right.

**So redundancy is a floor on the harm, not a detector of the fault.** What it
guarantees is that a collapsed interval costs the reader a distinction they can
still recover from the row's own text. What it cannot do is tell anyone the
interval collapsed.

**And that matters because the complaint this all came from was two
complaints.** `bdi-kbd2` records it as *"the inactive but not done rows are too
dark and similar to the completed rows"* — and that bead is the citation rather
than the origin: the sentence was transcribed into it from a pane turn, so
there is no message behind it to check the wording against. Stable and
addressable, which is what makes it the right thing to quote, and not
independent corroboration of itself.

- *similar to the completed rows* — redundancy answers this. The meaning
  survives, because `✓` carries it.
- ***too dark*** — redundancy does not touch this at all. A row can be
  perfectly redundant and still be unpleasant to read.

**The reason the second half is not a lesser complaint is a matter of area.**
The glyph is one cell and the row is eighty, so the eye takes the row's tone
before it takes the glyph. A scale that has collapsed *is* what the reader
experiences, whether or not anything has become ambiguous. Redundancy must
therefore never become the reason `bdi-kbd2` leaves the tones where they are.

`bdi-kbd2` had already reached half of this from the other end — its option 2
is *"stop carrying the bottom tier on brightness; a closed row already says so
in its own glyph"*, which is redundancy applied to one rung. What this section
adds is that the same move is a property of the whole scale, and that it buys
survivability rather than correctness.

**The recommendation, in order.**

1. **Redundancy now.** State it, and gate it with a test over the liveness
   states. Small, needs nothing, and it makes every future collapse
   recoverable — which is what the last two were not.
2. **`bdi-kbd2`, sized against both intervals and against a floor chosen in
   advance.** This is the one that fixes what the reader actually complained
   of, and redundancy does not substitute for it.
3. **(d) when something needs it.** Holding the theme is the only thing that
   closes the unknown case, and it is a great deal of machinery. Doing 2
   through a single resolving function — semantic name in, `Style` out — costs
   nothing now and is exactly the seam (d) would need later.

#### The scale is a ground and two rungs, not three rungs

**Which means what this document recommends is not option (b), and an earlier
draft said it was.** (b) is defined above as *every rung a palette slot, none
of them `Color::Reset`* — and the rule at the end of this section says the
default foreground must never be a rung. Those two only look compatible until
you try to write the code: a scale of three slots has to put one at the top,
the only slots near the top of a theme's range are 7 and 15, and (b)'s own
blind spot is that those vanish on a light theme. There is nowhere for a top
slot to go.

The resolution is to stop counting three rungs. **A staffed row is the
ground** — untreated, the terminal's default foreground, no colour spent on it
at all — and the scale is the two rungs *below* it. Then:

- there is nothing above the ground, which is not a limitation but the reason
  `bdi-sw4` could not find a colour to put there;
- the intervals to bound are ground-to-unstaffed and unstaffed-to-finished,
  which are exactly the two `bdi-kbd2` must size;
- and both of them are measured downward from a single reference, so neither
  is an interval between two values the terminal picked independently.

`bdi` already draws it this way. What was missing is the account of *why* that
is the right shape rather than an accident of `bdi-sw4`'s fix.

**Bold is ruled out here, and `bdi-kbd2` should only reach for it by
overturning answer 1 deliberately.** Two separate arguments land on it: the
channel assignment says weight is structural, and `bdi-sw4` measured the step
at a fifth of the one that works.

What this section adds is narrower, and it belongs on the record either way:
those two numbers were compared on one axis and are not the same kind of
quantity. 63.1% is a step that goes to zero when the theme changes, and on this
project's own theme it has, twice. 14.9% is a step no theme can move — only the
font, and only because there is no real Bold face at that variant. **A small
step with a floor and a large step with none are different offers**, and
nothing in the decision weighed that, because the table measured ink. If
`bdi-kbd2` finds the bottom interval still short, that is the trade to put in
front of Graeme — with the channel collision named, not buried.

**One rule falls out of all of it and is worth stating on its own:**

> **The terminal's default foreground is the ground a scale is measured
> against, never a rung on it.**

It is not a palette slot. It is an alias for whatever the theme chose, and it
is routinely the same value as slot 7 or slot 15 — which is not a broken theme,
it is how themes are written. Any scale with `Color::Reset` on it has an
interval nobody can bound, and `bdi-sw4` is what that looks like.

### 4. Whether `bdi` follows the reader's theme, and where

**It already does, almost everywhere it speaks in its own voice — and the
places it does not are a boundary rather than a gap.**

The split is exact, and it falls out of the mechanism in *What the tools
actually do* rather than out of anyone's intention:

| what | follows the theme | why |
|---|---|---|
| `Color::Reset` — staffed rows, `structure()`'s prefix, the window's head | **yes**, completely | it *is* the theme's default foreground |
| the named slots — `Green`, `Yellow`, `DarkGray`, `Cyan`, `White` | **yes** | emitted `\e[38;5;N`, so slots 0–15 are the reader's own configurable palette |
| `bd`'s four status literals (`DIM` among them), the id's blue | **no** | `Color::Rgb`, absolute, resolved by nobody |
| the background | **the reader's, always** | `bdi` sets none outside the selection's `REVERSED` |
| the tail band's replayed pane output | **the pane's program's choices** | `sgr.rs` reproduces what the program emitted, and that resolves against the reader's theme like anything else |

**The third row is not a defect and should be read as the boundary it is.**
`design.md:1476-1480` makes `bd`'s literals absolute on purpose, so that a
status is the same colour in `bdi` as it is in `bd`. So `bdi` follows the
reader's theme in everything it says in **its own** words, and follows nobody's
theme where it repeats **`bd`'s vocabulary**. That is a coherent rule, it is
already what the code does, and it has never been written down — which is why
`DIM`, an `Rgb` literal that is *not* part of `bd`'s vocabulary, drifted onto
the wrong side of it without anyone noticing. Answer 2's palette should make
the boundary explicit, because it is the thing that tells a future rung which
kind of value it may be.

**And `bdi` already obeys the one rule every program in the prior art obeys.**
It paints accents and never surfaces. That is worth stating as a rule it is
keeping rather than leaving as an accident of the current code, because it is
the single property that makes a light-theme reader's experience recoverable at
all.

#### Should it learn the reader's actual theme? Not now, and probably not this way

The only thing `bdi` would *do* with a queried theme is compute separation —
and answer 3 has just demoted that. Redundancy is the gate, separation is a
refinement, and buying a refinement with a terminal query is a poor trade at
this price:

- **The failure mode is a confident wrong answer, not an error.** Tabby answers
  OSC 11 with black regardless of its real background, and nothing downstream
  can catch that. delta's issue #1663 ran **from March 2024 to May 2026** with
  users on dark backgrounds getting the light theme, because `TERM=screen*` is
  excluded and a tmux built without the right flags reports `TERM=screen`.
- **It costs a flash or a double paint if it is not on the critical path, and
  blocks first paint if it is.** Neovim #32109 is open and milestoned for 0.12:
  the OSC 11 reply lands after the colorscheme has already applied under
  `background=dark`, so the scheme loads twice.
- **The classifiers do not agree on the question.** Neovim thresholds the
  background's luminance alone; `terminal-colorsaurus` compares foreground
  against background lightness. They return different answers on a low-contrast
  theme, which is the case that matters.
- **The cheap negative fails exactly where `bdi` lives.** A non-answer *is*
  distinguishable from a slow answer — send OSC 10/11 followed immediately by
  DA1, and a DA1 reply arriving first is a definite negative with no waiting.
  But that rests on the terminal preserving reply order, and a multiplexer is
  the documented exception: tmux did not preserve it until a November 2025 fix,
  and colorsaurus excludes `screen*` because screen replies out of order. `bdi`
  is read inside a multiplexer as its normal case — this seat's own pane reads
  `TERM=xterm-256color`, so the `screen*` heuristic would not even fire.

**And `bdi` has the shape of this hazard on file already, from the other
side.** `src/tui/clipboard.rs` writes OSC 52, and its own doc comment says
why that is safe: *"a terminal that does not honour it drops the sequence —
so the write can do nothing, and cannot fail for that."* A one-way OSC
degrades to nothing. A query does not: it degrades to a wait, or to an
answer that is wrong. The project has already reasoned this through for the
write and got it right; what it has not done is notice that the reasoning
does not carry to the read.

#### What to do instead: let the reader say

Take helix's spelling — a `[theme]` section with `dark`, `light` and a
`fallback` — and make it a config key.

**What it buys is the background axis, and that is exactly the axis that
matters.** Every failure in this document is a failure about the reader's
*background* rather than about their theme: slot 15 vanishes on a light ground,
faint inverts on a light ground, and a fixed grey is wrong on whichever ground
it was not tuned for. So a reader saying *my background is light* is telling
`bdi` the one fact all three turn on. That is a better fit than it looks — it
is the right axis rather than a serviceable approximation of a richer one.

- It cannot be wrong in a way the reader cannot fix, which is the whole of
  what goes wrong in delta #1663 — where the users' actual workaround was
  hardcoding `dark = true`, i.e. the thing the query was meant to replace.
- It costs no startup latency, no ordering assumption, and no raw-mode
  sequencing.

**What it does not buy is answer 3's contrast computation, and an earlier
draft of this section said it did.** A `dark`/`light` setting is a
classification, not a palette: it carries neither the default foreground nor
any slot's RGB, and many distinct themes classify the same while producing
different rung contrasts. Graeme's own two are the case in point — `noctalia`
and `dank-theme` are both dark, and `bdi-kbd2` records the bottom interval at
1.46:1 on one and 1.69:1 on the other. A key that cannot tell those apart
cannot compute either.

**The two are separate pieces of work and neither substitutes for the other:**

- **The corpus test needs a corpus, not a config key.** It is a table of real
  themes with their actual hexes, checked into this repository, asserted
  against at build time. It needs no config, no query and no reader — which is
  what answer 3 means by saying it exists whether or not `bdi` can ever ask a
  terminal anything.
- **Computing separation for the reader's own live theme is route (d)**, and
  the config key is not a cheap version of it. To get there the configuration
  would have to name a fully specified palette rather than a direction, at
  which point the reader is maintaining a second copy of their theme and the
  argument for querying starts to look better again. That trade is (d)'s to
  make, not this key's.

So the key is worth having on its own terms — it fixes what a wrong guess costs
a reader — and it is not the thing that makes the gate possible.

**What the fallback is, and whether a wrong one is detectable.** It should be
`dark`, because that is what `bdi`'s current palette is tuned for and a
default that matches the shipped tones is the one that changes nothing for
existing readers. **And yes — a wrong fallback is immediately visible to the
reader it is wrong for, which is the property that makes it acceptable and the
one a query does not have.**

That is worth being precise about, because "a guess that silently picks dark on
a light terminal reproduces the whole failure with a config key in front of it"
is the obvious objection and it is nearly right. Two things separate them, and
neither is about how often the guess is correct:

- **A wrong fallback is stable and attributable.** It is wrong the same way
  every time, on every terminal, from the first frame — so the reader sees it,
  and one documented key fixes it for good. A wrong *query* answer is
  intermittent: right in one terminal and wrong in another, right outside a
  multiplexer and wrong inside it. That is why delta #1663 took two years —
  not because the answer was wrong, but because nobody could see what was
  producing it.
- **A fallback makes no claim.** A reader who knows `bdi` guessed goes looking
  for the key. A reader who believes `bdi` *detected* their theme has no reason
  to think anything is adjustable, and will read the result as `bdi` being
  badly designed rather than as `bdi` being misconfigured.

So the honest form is a fallback that is documented as a guess. It fails
visibly, in one direction, with the remedy in the same paragraph as the
symptom.

`COLORFGBG` is the free half-measure — some terminals and multiplexers set it,
vim reads it, and it needs no query at all — but it is unset in this seat's own
environment, which is a fair sample of where `bdi` runs, so it is worth reading
where present and worth nothing to rely on.

**If a query is ever built, three constraints are already established and
should be carried into that bead rather than rediscovered.** It must run
**before raw mode is entered**, or crossterm's event reader swallows the reply.
It must be **paired with DA1** and bounded by a timeout, because the pairing is
what distinguishes a non-answer from a slow one, and the timeout is what covers
the multiplexer case where the pairing does not hold. And it must **never gate
the first paint** — draw on the configured default, and treat a late answer as
a reason to redraw, not a reason to wait.

## What this changes in `docs/design.md`

`design.md` is the spec. This review extends it in four places, contradicts it
in one, and the contradiction is stated here rather than left to be discovered.

**Extends — `design.md:1506-1512`, the two colour-carrying channels.** The
spec already argues that a bead row has exactly two, that the glyph's is `bd`'s
and the text's is liveness, and that priority goes undrawn because a third
would have to share one. Answer 1 is that argument applied to the whole screen
instead of one row. Nothing there is overturned.

**Extends — `design.md:1476-1480`, `bd`'s literals.** The spec says they are
literal on purpose, because `bd`'s are. Correct, and unchanged. What it does
not say is that `Color::Rgb` is emitted with no capability check, so those five
values plus the id's blue are a hard truecolor dependency. That is a sentence
the spec is missing rather than a sentence it has wrong.

**Extends — `design.md:1570`, the tail's dimming.** The spec makes it
load-bearing: it is *"the whole of what tells its words from the pane's"*.
Agreed, and that is exactly why it should not be a colour. Under `NO_COLOR` it
vanishes and takes the distinction with it. Making it an attribute keeps the
spec's claim true in a case the spec did not consider.

**Extends — the theme-following boundary, which the spec states nowhere.**
Answer 4 finds that `bdi` follows the reader's theme in everything it says in
its own words and follows nobody's theme where it repeats `bd`'s vocabulary.
That rule is already what the code does and it is a good rule; it is unwritten,
which is how `DIM` — an `Rgb` literal that is not part of `bd`'s vocabulary —
came to sit on the absolute side of it. The spec should carry the boundary, and
with it the rule every program in the prior art obeys and `bdi` already does:
**paint accents, never surfaces.**

**Contradicts — `design.md:1489-1490`, the top of the liveness scale.** The
spec says *"an agent on it keeps the terminal's default foreground"*, and this
review says the terminal's default must not be a rung on any scale.

**It is a reframing rather than a reversal, and the distinction is the whole
value of it.** Where the staffed row *sits* is not in dispute: the spec puts
it at the default foreground, this review puts it there too, and the code
already does. What is in dispute is whether that position is the **top rung
of a scale** or the **ground the scale hangs from**. The spec's own
reasoning was arrived at from `bdi-sw4`, where painting a staffed row
`White` collided with the default, and the fix — shift the scale down,
staffed stays at the default — is sound *as far as it goes*. The objection
is that it treats "nothing can sit above the default" as a fact about where
the scale should start, when it is a fact about what the default **is**: an
alias for whatever the theme chose, routinely equal to slot 7 or slot 15. A
scale with an alias on it has one interval nobody can bound, which is why
`bdi-kbd2` exists and why the top interval is 2.44:1 today. The spec's own
reasoning — *"a theme's default is already the brightest thing on its page"*
— is the argument for the default being the **ground**, not for it being the
top rung.

Read as the ground, three things stop being puzzles: there is nothing above it
because there is nothing above a reference; `bdi` has two rungs rather than
three, which is what the routes above find is the portable number; and both
intervals are measured downward from one value rather than between two the
terminal chose separately.

Two smaller things are **not** in the spec at all and so are not
contradictions, only new:

- The bead window drawing its head and its page at one tone, and carrying what
  stands out on a weight, is argued in `src/view/palette.rs` and nowhere in
  `design.md`.
- `Color::Reset` doubling as the mechanism for escaping a row's tone is a
  property of `Fitted`, undocumented in both.

**Where this document should be folded in.** `bdi-2bb.1` reconciles
`design.md` against what the code learned. This file is an input to that bead,
not a replacement for it: the spec should end up carrying the channel
assignment and the ground-not-a-rung rule in its own voice, and this file
should end up as the reading behind them rather than a second spec.

## Why a new file rather than a section of `design.md`

Both were defensible and this is the argument for the one chosen.

`design.md` is 2021 lines and already has a reconciliation bead of its own
open against it. Adding four hundred lines of survey, arithmetic and
alternatives-weighed would be adding to that debt with material that is
mostly *working*: the site tables, the contrast figures, the four routes
considered and every one of them found wanting. A spec says what is true;
almost none of this document is a statement of what `bdi` does.

The half that *is* — the channel assignment and the ground-not-a-rung rule —
belongs in `design.md` in the spec's own voice, and the section above says so.
Splitting it that way keeps `design.md` a spec and gives this reading somewhere
to live where it can be superseded without touching one.

## What this spawns, in order

Seven pieces of work fall out. The order matters: the first is the seam every
other one needs, and doing any of the others first means doing it twice.

**1. One palette, in one place.** Answer 2's list, as `Style`s named by role,
with every one of the twenty-odd production sites going through it and no site
spelling a `Color::` or a `Modifier::` of its own.

This one carries its own gate, and it is the cheap kind: **a check that
refuses a `Color::` or a `Modifier::` outside the palette module**, in the
shape of the `dead-code` and `screen-walks` checks the flake already runs.
`src/view/sgr.rs` is the one exemption, because it replays a pane's own
escapes rather than choosing anything. Without that check the palette drifts
back apart one convenient literal at a time, which is how it got to three sets
and six inline greys.

**2. The liveness scale, decided.** This is `bdi-kbd2`, and it is the first
thing this review's recommendation decides. It should be sized against both
intervals rather than the bottom one, and against a floor chosen in advance.

**3. The redundancy property, as a test.** Answer 3, and it is the piece that
changed most in the writing. The property to gate is *not* that adjacent rungs
are far enough apart — three portable rungs do not exist, so that gate cannot
be built — but that **no meaning rests on a scale interval alone**. A test over
the liveness states asserting that each is distinguishable from the row's own
text needs no terminal, no theme and no arithmetic.

It would **not** have caught either known defect — `◍` and `✓` were on those
rows throughout, so the gate would have been green while the tones collapsed.
It is a floor on the harm rather than a detector of the fault, and it must not
be read as a substitute for 2.

`Painted`'s `Run` holds the whole `Style` beside the words, and
`painted.rs:122` already asserts on `add_modifier.contains(Modifier::BOLD)`, so
the harness for this exists. Separation stays worth computing where `bdi` holds
both values; it is a refinement rather than the gate.

**4. The bead window stops spending brightness.** Head and page both at the
terminal's default, headings carried by weight. Small, and it is what makes
*dim* mean one thing screen-wide.

**5. The tail's voice becomes an attribute rather than a colour.** So that
`design.md:1570`'s claim survives `NO_COLOR`. This is what `bdi-7lie` is
about, and it should be decided by this review rather than beside it — but
*which* attribute is a real choice and neither candidate is free. Dim inverts
on a light theme on the terminals that multiply the foreground toward black,
and italic is absent from `screen-256color`'s terminfo, which is the classic
reason italics die in a multiplexer. The bead picks one knowing that.

**6. The reader gets to say what their theme is.** Answer 4: a `[theme]` key
with `dark`, `light` and a fallback, in helix's spelling. It carries the
background axis, which is the one fact every failure in this document turns on,
and it is what delta's users ended up doing by hand after two years of
detection. It is **not** what makes answer 3's contrast check possible — that
wants a corpus of real palettes checked in here, which is part of 3 and needs
no reader at all.

**7. `design.md` takes the surviving half.** The channel assignment, the
ground-not-a-rung rule and the theme-following boundary, in the spec's own
voice, through `bdi-2bb.1`. This file stays as the reading behind them.

Numbers 1 and 3 are the ones that make the difference. The rest are point
fixes, and point fixes are what the last six palette changes were.


## Sources

The three sections that read outside this codebase, cited. Everything else in
this document is read from `bdi` at `22acc0d` and is cited in place.

**ratatui and crossterm** were read from the source of ratatui `main` /
`ratatui-crossterm` 0.1.2 over crossterm 0.29, at the paths named where each
claim appears — `ratatui_core::style::color`, `ratatui_core::style::palette`,
crossterm's `style.rs` and `style/types/colored.rs`, and ratatui's `demo2`
example. `NO_COLOR` is [no-color.org](https://no-color.org/).

**What terminals do with the intensity attribute**

- SGR 2 support and per-terminal mechanism —
  [TerminalGuide attr/2](https://terminalguide.namepad.de/attr/2/)
- Windows Terminal gained faint —
  [microsoft/terminal#6703](https://github.com/microsoft/terminal/issues/6703)
- Faint is darkest on a light background —
  [microsoft/terminal#16493](https://github.com/microsoft/terminal/issues/16493)
- ConPTY's SGR 3 swallowing, fixed 2019 —
  [microsoft/terminal#2554](https://github.com/microsoft/terminal/issues/2554)
- Faint parsed but never applied, since fixed —
  [zed-industries/zed#7497](https://github.com/zed-industries/zed/issues/7497)
- `dim` is not gated behind `terminal-features` —
  [tmux terminal-features](https://tmux-tmux.mintlify.app/advanced/terminal-features)
- iTerm2 and xterm.js shift toward the background —
  [muesli/termenv#45](https://github.com/muesli/termenv/issues/45)
- Dim with reverse fails on wezterm, GNOME Terminal and Foot —
  [helix-editor/helix#11009](https://github.com/helix-editor/helix/discussions/11009)
- Bold changes the font face and not the colour —
  [kovidgoyal/kitty#512](https://github.com/kovidgoyal/kitty/issues/512)
- Brightening does not apply to the default foreground —
  [wezterm `bold_brightens_ansi_colors`](https://wezterm.org/config/lua/config/bold_brightens_ansi_colors.html)
- `boldColors`, default true — xterm's manpage
- `intenseTextStyle`, default `"bright"` — Windows Terminal settings docs
- `dim_opacity` default 0.4 — kitty `kitty/options/definition.py`
- `faint-opacity` default 0.5 — Ghostty `src/config/Config.zig`
- `DIM_FACTOR = 0.66` — Alacritty `alacritty/src/display/color.rs`
- `NamedColor::Foreground` falls through the bright path — Alacritty
  `alacritty/src/display/content.rs`, `compute_fg_rgb`

**What other programs do about a theme they cannot see**

- Faint, bold and `GIT_COLOR_FAINT_DEFAULT` —
  [git `color.h`](https://github.com/git/git/blob/master/color.h); the dual
  colouring is `git range-diff`'s documented behaviour
- *"conservative that work across terminal themes"* — ripgrep
  `crates/printer/src/color.rs`
- `bold, underline, reverse, strikethrough` and no dim — lazygit
  `pkg/gui/style/decoration.go`
- The light theme removed —
  [lazygit v0.36.0](https://github.com/jesseduffield/lazygit/releases/tag/v0.36.0)
- Explicit `White` replaced by `dimmed()` —
  [eza-community/eza#1406](https://github.com/eza-community/eza/issues/1406)
- Default theme chosen by colour depth — fzf `src/tui/light_unix.go`
- *"simultaneously increase and decrease the intensity"* —
  [wezterm#4026](https://github.com/wezterm/wezterm/discussions/4026)
- helix's `term16_*` themes and `[theme] dark/light/fallback` — helix's own
  theme files and configuration docs

**Detection, and how it fails**

- Wrong theme for two years under `TERM=screen` —
  [dandavison/delta#1663](https://github.com/dandavison/delta/issues/1663)
- The colorscheme loads twice because the reply lands late —
  [neovim/neovim#32109](https://github.com/neovim/neovim/issues/32109)
- OSC 11 answering black regardless of the real background —
  [Eugeny/tabby#10121](https://github.com/Eugeny/tabby/issues/10121)
- Reply order not preserved, fixed November 2025 —
  [tmux/tmux#4681](https://github.com/tmux/tmux/issues/4681)
- The tested terminal set, the `screen*` exclusion and the 1-second timeout —
  [terminal-colorsaurus `terminal-survey.md`](https://github.com/bash/terminal-colorsaurus/blob/main/doc/terminal-survey.md)
- Pairing the query with DA1 as a cheap negative —
  [terminal-colorsaurus `feature-detection.md`](https://github.com/bash/terminal-colorsaurus/blob/main/doc/feature-detection.md)

**What is deliberately not cited**, because the reading that produced it
flagged these as inferred rather than sourced and they are excluded from the
argument above: git's rendering of hunk headers on light backgrounds; htop's
rationale for its "Broken Gray" scheme; whether Apple Terminal's *"Use bright
colors for bold text"* is on by default; that the indexed-remap argument
generalises to konsole, urxvt and Windows Terminal, which were not checked
individually on the default-foreground case; and the *absence* of background
detection in git, fzf, gitui, lazygit, k9s, btop, eza and lsd, which was
established by reading their colour and theme code rather than exhaustively,
so a strong negative rather than a proof.
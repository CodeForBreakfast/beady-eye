#!/usr/bin/env python3
"""Replay a raw `bdi` capture onto a grid, and write it out as text or SVG.

Replayed rather than stripped of its escapes: `bdi` draws over itself, so the
stream read as a sequence of glyphs is not the screen a terminal ends up
showing. The grid is.

SGR is differential — crossterm emits only what changed, so an attribute set
once and never reset is in force over every cell drawn after it. The style is
therefore a dict each parameter mutates, and every cell keeps a copy of the
style in force when it was written. A replayer that took each `m` as a whole
style would lose exactly the bold that says a row is staffed.
"""

import argparse
import re
import sys
from xml.sax.saxutils import escape, unescape

# xterm's first sixteen, as a terminal with a dark background draws them.
# Chosen once here so the picture has a palette of its own rather than
# whatever the reader's terminal would have applied.
BASE16 = [
    "#1c1f26", "#e05252", "#4fa65b", "#c9a227",
    "#4a8fe0", "#a86fd0", "#3fa8a0", "#c3c8d1",
    "#5c6370", "#f07171", "#6cc46c", "#e0c14a",
    "#6fa8f0", "#c48fe0", "#5fc8c0", "#eef1f6",
]
BACKGROUND = "#12151a"
FOREGROUND = "#c3c8d1"


def cube(n):
    """xterm's 256-colour cube and greyscale ramp."""
    if n < 16:
        return BASE16[n]
    if n < 232:
        n -= 16
        levels = [0, 95, 135, 175, 215, 255]
        r, g, b = levels[n // 36], levels[(n // 6) % 6], levels[n % 6]
        return f"#{r:02x}{g:02x}{b:02x}"
    v = 8 + (n - 232) * 10
    return f"#{v:02x}{v:02x}{v:02x}"


def blank():
    return {"fg": None, "bg": None, "bold": False, "dim": False, "reverse": False}


class Grid:
    def __init__(self, rows, cols):
        self.rows, self.cols = rows, cols
        self.text = [[" "] * cols for _ in range(rows)]
        self.style = [[blank() for _ in range(cols)] for _ in range(rows)]
        self.r = self.c = 0
        self.cur = blank()

    def put(self, ch):
        if 0 <= self.r < self.rows and 0 <= self.c < self.cols:
            self.text[self.r][self.c] = ch
            self.style[self.r][self.c] = dict(self.cur)
        self.c += 1

    def sgr(self, params):
        it = iter(params)
        for p in it:
            if p in ("", "0"):
                self.cur = blank()
            elif p == "1":
                self.cur["bold"] = True
            elif p == "2":
                self.cur["dim"] = True
            elif p == "7":
                self.cur["reverse"] = True
            elif p == "22":
                self.cur["bold"] = self.cur["dim"] = False
            elif p == "27":
                self.cur["reverse"] = False
            elif p == "39":
                self.cur["fg"] = None
            elif p == "49":
                self.cur["bg"] = None
            elif p in ("38", "48"):
                where = "fg" if p == "38" else "bg"
                nxt = next(it, None)
                if nxt == "5":
                    self.cur[where] = cube(int(next(it, "0")))
                elif nxt == "2":
                    rgb = [int(next(it, "0")) for _ in range(3)]
                    self.cur[where] = "#%02x%02x%02x" % tuple(rgb)
            elif p.isdigit():
                n = int(p)
                if 30 <= n <= 37:
                    self.cur["fg"] = BASE16[n - 30]
                elif 90 <= n <= 97:
                    self.cur["fg"] = BASE16[n - 90 + 8]
                elif 40 <= n <= 47:
                    self.cur["bg"] = BASE16[n - 40]
                elif 100 <= n <= 107:
                    self.cur["bg"] = BASE16[n - 100 + 8]

    def clear(self, r, c):
        self.text[r][c] = " "
        self.style[r][c] = blank()


CSI = re.compile(rb"\x1b\[([0-9;?]*)([a-zA-Z])")
OSC = re.compile(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


def replay(raw, rows, cols):
    g = Grid(rows, cols)
    i, n = 0, len(raw)
    while i < n:
        if raw[i] == 0x1B:
            m = OSC.match(raw, i)
            if m:
                i = m.end()
                continue
            m = CSI.match(raw, i)
            if not m:
                i += 2
                continue
            args = m.group(1).decode().split(";")
            verb = m.group(2)
            if verb == b"m":
                g.sgr(args)
            elif verb == b"H":
                g.r = int(args[0] or 1) - 1
                g.c = int(args[1] or 1) - 1 if len(args) > 1 else 0
            elif verb == b"J" and (args[0] or "0") == "2":
                for r in range(g.rows):
                    for c in range(g.cols):
                        g.clear(r, c)
            elif verb == b"K":
                if 0 <= g.r < g.rows:
                    for c in range(g.c, g.cols):
                        g.clear(g.r, c)
            elif verb == b"A":
                g.r -= int(args[0] or 1)
            elif verb == b"B":
                g.r += int(args[0] or 1)
            elif verb == b"C":
                g.c += int(args[0] or 1)
            elif verb == b"D":
                g.c -= int(args[0] or 1)
            i = m.end()
            continue
        ch = raw[i : i + 1]
        if ch == b"\r":
            g.c = 0
            i += 1
        elif ch == b"\n":
            g.r += 1
            g.c = 0
            i += 1
        else:
            length = 4 if raw[i] >= 0xF0 else 3 if raw[i] >= 0xE0 else 2 if raw[i] >= 0xC0 else 1
            try:
                glyph = raw[i : i + length].decode("utf-8")
            except UnicodeDecodeError:
                glyph = "?"
            if glyph.isprintable() or glyph == " ":
                g.put(glyph)
            i += length
    return g


def painted(style):
    """The colours a cell is actually drawn in, with reverse applied."""
    fg = style["fg"] or FOREGROUND
    bg = style["bg"] or BACKGROUND
    if style["reverse"]:
        fg, bg = bg, fg
    return fg, bg


def runs(g, r):
    """The row as (column, text, fg, bg, bold, dim) runs of one style."""
    out = []
    start = 0
    while start < g.cols:
        fg, bg = painted(g.style[r][start])
        bold, dim = g.style[r][start]["bold"], g.style[r][start]["dim"]
        end = start + 1
        while end < g.cols:
            nfg, nbg = painted(g.style[r][end])
            if (nfg, nbg, g.style[r][end]["bold"], g.style[r][end]["dim"]) != (
                fg, bg, bold, dim
            ):
                break
            end += 1
        out.append((start, "".join(g.text[r][start:end]), fg, bg, bold, dim))
        start = end
    return out


def as_text(g):
    return "\n".join("".join(g.text[r]).rstrip() for r in range(g.rows))


def as_svg(g, font_size, line_height, title):
    cell = font_size * 0.6
    width = round(g.cols * cell, 2)
    height = round(g.rows * line_height, 2)
    pad = round(line_height * 0.6, 2)

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width + 2 * pad:.2f}" '
        f'height="{height + 2 * pad:.2f}" '
        f'viewBox="0 0 {width + 2 * pad:.2f} {height + 2 * pad:.2f}" '
        f'font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, '
        f'&quot;DejaVu Sans Mono&quot;, monospace" '
        f'font-size="{font_size}" role="img" aria-label="{escape(title, {chr(34): "&quot;"})}">',
        f"<title>{escape(title)}</title>",
        f'<rect width="100%" height="100%" rx="{pad:.2f}" fill="{BACKGROUND}"/>',
    ]

    # Backgrounds first, as whole rectangles, so a run that carries one is not
    # a row of coloured spaces the text layer would have to draw over.
    for r in range(g.rows):
        for col, text, _fg, bg, _bold, _dim in runs(g, r):
            if bg == BACKGROUND:
                continue
            out.append(
                f'<rect x="{pad + col * cell:.2f}" y="{pad + r * line_height:.2f}" '
                f'width="{len(text) * cell:.2f}" height="{line_height:.2f}" fill="{bg}"/>'
            )

    for r in range(g.rows):
        baseline = pad + r * line_height + line_height * 0.75
        for col, text, fg, _bg, bold, dim in runs(g, r):
            if not text.strip():
                continue
            # Every glyph carries its own x, so the picture is a grid whatever
            # monospace font the reader's browser resolves — and whether it
            # resolves one at all. A run given a single x and a `textLength`
            # is the obvious alternative and is not equivalent: the browser
            # honours the length by stretching glyphs, so a row falls out of
            # column against the row above it as soon as a fallback font's
            # advance differs from the one assumed here.
            lead = len(text) - len(text.lstrip(" "))
            body = text.strip(" ")
            xs = " ".join(
                f"{pad + (col + lead + n) * cell:.2f}" for n in range(len(body))
            )
            # Without this the XML parser folds each run of spaces into
            # one, so a row with a gap in it has fewer glyphs than the x-list
            # has positions and everything after the gap slides left.
            attrs = [
                f'x="{xs}"',
                f'y="{baseline:.2f}"',
                f'fill="{fg}"',
                'xml:space="preserve"',
            ]
            if bold:
                attrs.append('font-weight="700"')
            if dim:
                attrs.append('opacity="0.6"')
            out.append(f"<text {' '.join(attrs)}>{escape(body)}</text>")

    out.append("</svg>")
    return "\n".join(out)


def reread(svg, rows, cols, font_size, line_height):
    """The SVG's own glyphs, back onto a grid.

    Read back rather than trusted. What decides whether the picture is the
    screen is where each glyph lands, and every way of writing a run of text
    into SVG puts that decision somewhere a diff of the file will not show:
    an x-list one short of its glyphs, a run of spaces the parser folded, a
    row rounded onto its neighbour. All three leave a well-formed SVG that
    renders, and none of them survives being replayed and compared.
    """
    cell = font_size * 0.6
    pad = round(line_height * 0.6, 2)
    grid = [[" "] * cols for _ in range(rows)]
    for m in re.finditer(r'<text x="([^"]+)" y="([^"]+)"[^>]*>(.*?)</text>', svg):
        xs = [float(v) for v in m.group(1).split()]
        body = unescape(m.group(3))
        if len(xs) != len(body):
            raise AssertionError(f"{len(xs)} positions for {len(body)} glyphs: {body!r}")
        row = round((float(m.group(2)) - pad - line_height * 0.75) / line_height)
        for x, ch in zip(xs, body):
            grid[row][round((x - pad) / cell)] = ch
    return "\n".join("".join(r).rstrip() for r in grid)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("capture")
    ap.add_argument("--rows", type=int, required=True)
    ap.add_argument("--cols", type=int, required=True)
    ap.add_argument("--svg", help="write the frame here as SVG")
    ap.add_argument("--text", help="write the frame here as plain text")
    ap.add_argument("--font-size", type=float, default=13.0)
    ap.add_argument("--line-height", type=float, default=17.0)
    ap.add_argument("--title", default="A bdi frame")
    args = ap.parse_args()

    g = replay(open(args.capture, "rb").read(), args.rows, args.cols)
    if args.text:
        open(args.text, "w").write(as_text(g) + "\n")
    if args.svg:
        svg = as_svg(g, args.font_size, args.line_height, args.title)
        open(args.svg, "w").write(svg + "\n")
        back = reread(svg, args.rows, args.cols, args.font_size, args.line_height)
        if back != as_text(g):
            for wrote, drawn in zip(back.split("\n"), as_text(g).split("\n")):
                if wrote != drawn:
                    print(f"svg |{wrote}|\ncap |{drawn}|", file=sys.stderr)
            sys.exit("the SVG is not the frame that was captured")
    if not args.svg and not args.text:
        sys.stdout.write(as_text(g) + "\n")


if __name__ == "__main__":
    main()

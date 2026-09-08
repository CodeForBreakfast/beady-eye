# Security policy

## Reporting a vulnerability

Report a suspected vulnerability privately. **Don't open a public issue** — that
publishes the flaw before there is a fix.

Use GitHub's [private vulnerability reporting][pvr]: open this repository's
**Security** tab and choose **Report a vulnerability**. That opens an advisory
only you and the maintainers can see. If you would rather not use GitHub, email
**info@codeforbreakfast.co** instead.

Say enough to reproduce it: the commit you saw it at, the `bd` and `herdr`
versions in play, the config you were running under with any credentials taken
out, and what you observed. There is no SLA — this is a small project — but we
will acknowledge the report, keep you posted, and credit you in the advisory
unless you ask us not to.

## Which versions get fixes

`bdi` is unreleased. There is no tag and nothing on crates.io, so report against
`main` and name the commit. Once there are releases, only the latest tag will
carry fixes.

## What is worth reporting

`bdi` issues only reads: every `bd` command line it spells is a read, and the
pane tail it draws is output it was handed rather than a shell it runs. That is
a property of those command lines rather than a guarantee about your tracker —
`bd` writes on its own account when it opens one, rewriting
`.beads/.local_version` and migrating the schema before it runs whatever it was
asked for. [docs/configuration.md](docs/configuration.md#each-tracker-read-by-its-own-bd) has that, and it is not a
vulnerability.

The places worth pointing a report at are where the reading stops being the
whole story.

**It runs commands its config names.** `environment_command` and
`credential_command` are run per project, and a directory holding an `.envrc` is
entered with `direnv exec .`. The config file is as trusted as the account that
can write it. What would be a vulnerability is a way to reach any of those
commands from something that is *not* the config — a tracker's contents, a
pane's output, the change socket.

**It handles a tracker password.** A `credential_command`'s output is captured
rather than put on a command line, so it stays out of `ps`. Anywhere it reaches a
process argument, an environment a child did not need, the screen, or `--json` is
worth a report.

**It listens on a socket.** `$XDG_RUNTIME_DIR/beady-eye/changes.sock` is created
mode `0600` under the user's own runtime directory, and its whole protocol is one
project name per line. Anything that lets a line do more than schedule a read of
a project the config already names belongs here.

**It draws what other programs say.** Bead titles, bead metadata and pane output
all come from outside `bdi` and end up on a terminal. Content that escapes the
region it is drawn in, or that reaches the terminal as control sequences rather
than as text, is in scope.

A flaw in the tracker itself belongs with [beads][beads], and one in the
multiplexer with [herdr][herdr].

[pvr]: https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability
[beads]: https://github.com/gastownhall/beads
[herdr]: https://herdr.dev

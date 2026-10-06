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

`bdi` issues reads, with two exceptions. `bdi bd <project> human respond` runs
that write against the named project's tracker, and `bdi gates` settles gh:pr
gates. Every other `bd` command line it spells is a read, and the pane tail it draws is output it was handed rather
than a shell it runs. That is a property of those command lines rather than a
guarantee about your tracker —
a `bd` older than 1.3.0 writes on its own account when it opens one, rewriting
`.beads/.local_version` and migrating the schema before it runs whatever it was
asked for, even under `--readonly`. [docs/configuration.md](docs/configuration.md#each-tracker-read-by-its-own-bd) has that, and it is not a
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

**It takes connections on a socket.** `$XDG_RUNTIME_DIR/beady-eye/changes.sock` is created
mode `0600` under the user's own runtime directory, and its whole protocol is one
project name per line. Anything that lets a line do more than schedule a read of
a project the config already names belongs here.

**It passes one write through.** `bdi bd` runs `bd human respond` against the
tracker of the project it names, with that project's credential. A command line
that gets it to run anything else, or to reach another project's tracker, is
worth a report.

**It settles pull-request gates.** `bdi gates` runs `gh pr view` on the
repository and number a gh:pr gate names, as whoever `gh` is signed in as. On
GitHub's word that the pull request merged, it runs `bd gate resolve` on that
gate. On its word that the pull request closed unmerged, it runs `bd comments
add` on each bead the gate holds back. Each runs against a configured project's
tracker, with that project's credential. A gate's `repo` and await id come from
the tracker, so anyone who can write to a tracker can point `bdi gates` at a
pull request. Anything that gets it to close a gate whose pull request did not
merge, to write to any other bead, to run any other command, or to reach a
tracker the config does not name is worth a report.

**It takes GitHub's deliveries over HTTP.** `bdi gates --listen` answers on a
network address, which may face the internet, and it is the one part of `bdi`
that does. It speaks plain HTTP and expects TLS to be ended in front of it. It
reads at most 1 MiB of a body before checking its signature, gives each request
ten seconds to arrive, answers at most eight at once, and keeps at most 64
signed deliveries waiting to be settled. A sender that holds it at the limit of
eight delays deliveries until the next look, which settles them anyway, so that
alone is not a vulnerability. Anything that lets a sender without the secret
take one of the 64 places is. A delivery is taken only where its
`X-Hub-Signature-256` is the HMAC-SHA256 of its body under the configured
secret, compared in constant time, and only a `pull_request` delivery is acted
on. All it reads from one is a repository and a number, which it settles
exactly as a gate naming them would be settled:
GitHub is asked where that pull request stands, and only the gates a
configured tracker already holds for it are touched. The secret is read from a
file or the environment, and taken out of the environment before any `bd` or
`gh` starts. Anything that gets an unsigned or wrongly signed request past the
check, gets a delivery to do more than settle the pull request it names, or
gets the secret out of the process is worth a report.

**It draws what other programs say.** Bead titles, bead metadata and pane output
all come from outside `bdi` and end up on a terminal. Content that escapes the
region it is drawn in, or that reaches the terminal as control sequences rather
than as text, is in scope.

A flaw in the tracker itself belongs with [beads][beads], and one in the
multiplexer with [herdr][herdr].

[pvr]: https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability
[beads]: https://github.com/gastownhall/beads
[herdr]: https://herdr.dev

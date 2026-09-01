{
  description = "beady-eye - a bead graph annotated with live herdr pane activity";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Beads ships its own flake. Do not add inputs.nixpkgs.follows here — beads
    # needs Go 1.26 and this flake's nixpkgs carries an older toolchain.
    #
    # Every check builds bd, so this pin is not maintainer-only the way it is in
    # a repository that keeps bd out of its contributor path. It is also the
    # schema version the maintainers' tracker was created at, so moving it moves
    # that store — treat it as a schema decision, not a version bump.
    beads.url = "github:gastownhall/beads/v1.2.2";
    # The dependency build is its own derivation here, and crane is what makes
    # one. It declares no inputs of its own, so it costs a lock entry and
    # nothing else.
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, beads, crane }:
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

      # The crate names the version once. A release tag that disagrees with it
      # is refused before anything is published, so a crate on crates.io always
      # has a flake output built from the same source at the same version.
      common = {
        pname = cargoToml.package.name;
        version = cargoToml.package.version;
        src = ./.;
      };

      # The dependency graph, compiled on its own and keyed on Cargo.lock rather
      # than on the source. Nothing in this repository changes it, so the store
      # and the shared cache can hold one across every later run and every later
      # check — which is the whole point, because compiling it is most of what
      # this project waits for.
      #
      # Two of them, because a check reuses artifacts only at the profile it was
      # built at, and the checks are not all at one profile: the package builds
      # and tests at release, while clippy, the dead-code pass and `cargo
      # package`'s verify build all run at dev. Building both is what leaves
      # every check's command exactly as it was.
      artifactsFor = pkgs:
        let craneLib = crane.mkLib pkgs; in {
          release = craneLib.buildDepsOnly common;
          dev = craneLib.buildDepsOnly (common // {
            pname = "${common.pname}-dev";
            CARGO_PROFILE = "dev";
          });
        };

      # The overlay and the per-system outputs are the same package, so a
      # consumer taking either gets what CI built.
      beadyEyeFor = pkgs: (crane.mkLib pkgs).buildPackage (common // {
        cargoArtifacts = (artifactsFor pkgs).release;

        # tests/no_config.rs runs the binary as a fresh machine would, and
        # bdi asks bd where the tracker is. The worktree-listing test builds
        # a repository and adds a worktree to it, so git has to be here too.
        # Only the check phase needs either; nothing at runtime is built
        # against them.
        nativeCheckInputs = [
          beads.packages.${pkgs.stdenv.hostPlatform.system}.bd
          pkgs.git
        ];

        # The package is named for the crate, the binary for the command.
        meta.mainProgram = "bdi";
      });
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        beady-eye = beadyEyeFor pkgs;
        artifacts = artifactsFor pkgs;

        # Starts bdi and puts it back whenever the source changes, so a copy
        # left running in a terminal keeps up with what the other seats land.
        #
        # `--wrap-process=none` and the `exec` are load-bearing together.
        # watchexec's default runs the command in a process group of its own,
        # which is a background group on the terminal, and entering raw mode
        # from a background group raises SIGTTOU: bdi stops before it draws
        # anything. Sharing watchexec's own group lifts that — but then the
        # signal that restarts goes to the process watchexec spawned, so that
        # process has to be bdi itself rather than a `cargo run` holding it as
        # a child it would not pass the signal on to.
        #
        # It watches the whole of src/, so a test-only edit restarts the view
        # too. The tests sit in `#[cfg(test)]` modules inside the files they
        # cover, and a filesystem event says which file changed, never which
        # part of it, so nothing can separate them. A restart nobody asked for
        # costs a redraw; a rebuild that never happens leaves the view showing
        # yesterday's build, which is what this exists to prevent. tests/ is
        # left out: an edit there cannot change what the running binary does.
        rerunBdiOnChange = pkgs.writeShellScriptBin "rerun-bdi-on-change" ''
          cd "$(${pkgs.git}/bin/git rev-parse --show-toplevel)" || exit 1
          exec ${pkgs.watchexec}/bin/watchexec \
            --watch src --watch Cargo.toml --restart --wrap-process=none \
            -- 'cargo build --quiet && exec target/debug/bdi'
        '';

        # `nix flake check` reads the git index, so an untracked file is not in
        # the source it checks and a green result has not compiled it. Refusing
        # the tree the check cannot see all of is what puts that out of reach.
        checkBeforePush = pkgs.writeShellScriptBin "check-before-push" ''
          set -u

          git=${pkgs.git}/bin/git

          cd "$($git rev-parse --show-toplevel)" || exit 1

          dirty="$($git status --porcelain)"
          if [ -n "$dirty" ]; then
            echo "check-before-push: this tree is dirty, so a check of it would not"
            echo "be a check of what you are about to push."
            echo
            printf '%s\n' "$dirty"

            untracked="$($git ls-files --others --exclude-standard)"
            if [ -n "$untracked" ]; then
              echo
              echo "These are untracked, so the check would not see them at all:"
              printf '%s\n' "$untracked"
            fi

            echo
            echo "Commit, then run this again."
            exit 1
          fi

          exec nix flake check -L "$@"
        '';

        # `gh run list` answers with an empty list for four different reasons
        # and only one of them means "wait", so this has to tell them apart —
        # `read-ci-verdict --help` says how.
        #
        # The fetch is load-bearing. Ancestry measured against a stale
        # `origin/main` calls a superseded commit the tip and sends you back to
        # wait for a run that will never exist, which is the wrong answer this
        # command exists to prevent.
        readCiVerdict = pkgs.writeShellScriptBin "read-ci-verdict" ''
          set -u

          git=${pkgs.git}/bin/git
          gh=${pkgs.gh}/bin/gh
          jq=${pkgs.jq}/bin/jq
          grep=${pkgs.gnugrep}/bin/grep

          case "''${1:-}" in
            -h|--help)
              cat <<'USAGE'
          read-ci-verdict [<commit-ish>]        (default: HEAD)

          Says whether CI passed for one commit, and exits 0 only for a run
          whose own conclusion is "success".

          GitHub makes one run per push, on the tip, so an empty run list is not
          a verdict and does not always mean wait:

            no run coming       CI runs on pushes to main and on pull requests
                                against it. A commit that is on neither has no
                                run and will never get one.
            conflicted          A pull request is run on a merge of its head
                                and its base, so one GitHub cannot merge gets
                                no run until somebody resolves it.
            not the tip         The verdict belongs to a descendant. This reads
                                that run instead and says whose it is, and a
                                green one that contains your commit exits 0:
                                a run tests a tree rather than a commit, no
                                run will ever test yours alone, and this is
                                the strongest true claim available. It may be
                                green because of what landed after you.
            not started yet     Wait. This one resolves on its own.

          A cancelled run is neither green nor red: the commit has no verdict
          and the run wants starting again. Why runs get cancelled here has not
          been established, so read no cause into one.
          USAGE
              exit 0
              ;;
          esac

          cd "$($git rev-parse --show-toplevel)" || exit 1

          ref="''${1:-HEAD}"
          sha="$($git rev-parse --verify --quiet "$ref^{commit}")" || {
            echo "read-ci-verdict: no such commit in this repository: $ref" >&2
            exit 1
          }

          runs_for() {
            $gh run list --commit "$1" --limit 50 \
              --json databaseId,headSha,status,conclusion,workflowName,url,createdAt
          }

          runs="$(runs_for "$sha")" || exit 1
          subject="$sha"

          if [ "$(printf '%s' "$runs" | $jq 'length')" -eq 0 ]; then
            $git fetch --quiet origin ||
              echo "read-ci-verdict: could not reach origin, so what follows may be stale" >&2

            # Silence is only readable against a known trigger, and this reads
            # it against one. Say so rather than answer for a workflow this no
            # longer describes.
            workflow="$($git show "$sha:.github/workflows/ci.yml" 2>/dev/null)"
            if ! printf '%s\n' "$workflow" | $grep -q 'branches: \[main\]' ||
               printf '%s\n' "$workflow" | $grep -qE 'paths-ignore|^ *paths:'; then
              echo "NO VERDICT — $sha has no run, and this cannot say why."
              echo "ci.yml no longer triggers on a bare push to main, so silence can"
              echo "now mean a filtered path too. Teach this command the new trigger."
              exit 1
            fi

            if ! $git merge-base --is-ancestor "$sha" origin/main 2>/dev/null; then
              pr="$($gh pr list --state open --limit 100 \
                  --json headRefOid,url,mergeable |
                $jq -r --arg sha "$sha" \
                  'first(.[] | select(.headRefOid == $sha)) |
                   "\(.mergeable) \(.url)"')"
              if [ -n "$pr" ]; then
                url="''${pr#* }"

                # GitHub builds a pull request's run on a merge of the head and
                # the base, so a conflict leaves it with nothing to run and the
                # wait never ends on its own. UNKNOWN is a mergeability it has
                # not computed yet, which does.
                if [ "''${pr%% *}" = CONFLICTING ]; then
                  echo "NO RUN UNTIL YOU RESOLVE IT — $sha heads a pull request that"
                  echo "conflicts with its base, and GitHub runs nothing it cannot merge."
                  echo "  $url"
                  echo "Merge origin/main, resolve, and push. The run follows the push."
                  exit 1
                fi

                echo "NOT STARTED YET — $sha heads an open pull request and has no run."
                echo "  $url"
                echo "This one resolves on its own. Ask again."
                exit 1
              fi

              echo "NO RUN, AND NONE IS COMING — $sha is neither on origin/main nor"
              echo "the head of an open pull request, and CI runs on those two things"
              echo "only. Open a pull request and a run appears."
              exit 1
            fi

            tip="$($git rev-parse origin/main)"
            if [ "$tip" = "$sha" ]; then
              echo "NOT STARTED YET — $sha is the tip of origin/main and has no run."
              echo "This one resolves on its own. Ask again."
              exit 1
            fi

            echo "No run for $sha, and there never will be:"
            echo "it is on origin/main but not the tip, and GitHub makes one run per"
            echo "push. Its verdict lives on the descendant that contains it:"
            echo "  $tip"
            echo
            subject="$tip"
            runs="$(runs_for "$tip")" || exit 1
            if [ "$(printf '%s' "$runs" | $jq 'length')" -eq 0 ]; then
              echo "NO VERDICT — $tip has no run either. Ask again once it does."
              exit 1
            fi
          fi

          stray="$(printf '%s' "$runs" |
            $jq --arg sha "$subject" '[.[] | select(.headSha != $sha)] | length')"
          if [ "$stray" != "0" ]; then
            echo "read-ci-verdict: gh returned a run for another commit; refusing to guess." >&2
            exit 1
          fi

          latest="$(printf '%s' "$runs" |
            $jq -c 'group_by(.workflowName) | map(max_by(.createdAt))')"

          printf '%s' "$latest" |
            $jq -r '.[] | "  \(.workflowName): \(.status)/\(if .conclusion == null or .conclusion == "" then "-" else .conclusion end)  \(.url)"'
          echo

          verdict="$(printf '%s' "$latest" | $jq -r '
            if   any(.[]; .status != "completed") then "running"
            elif any(.[]; .conclusion == "failure" or .conclusion == "timed_out"
                          or .conclusion == "startup_failure"
                          or .conclusion == "action_required") then "failed"
            elif any(.[]; .conclusion == "cancelled") then "cancelled"
            elif all(.[]; .conclusion == "skipped") then "skipped"
            elif all(.[]; .conclusion == "success") then "passed"
            else "unclear" end')"

          case "$verdict" in
            passed)
              echo "PASSED — $subject"
              ;;
            failed)
              echo "FAILED — $subject"
              exit 1
              ;;
            cancelled)
              echo "NO VERDICT — a run for $subject was cancelled."
              echo "Cancelled is neither green nor red. Start it again."
              exit 1
              ;;
            skipped)
              echo "NO VERDICT — every run for $subject was skipped, so nothing was checked."
              exit 1
              ;;
            running)
              echo "STILL RUNNING — $subject has no conclusion yet."
              exit 1
              ;;
            *)
              echo "NO VERDICT — $subject's runs concluded in a way this command does"
              echo "not recognise. Read them above."
              exit 1
              ;;
          esac
        '';

        # A guard nobody has watched fire is the shape this project keeps
        # finding, and the dirty-tree refusal is the one guard here that is CI
        # correctness rather than workflow: a green check of a tree nix cannot
        # see all of is a false green.
        checkBeforePushTest = pkgs.runCommand "check-before-push-test"
          { nativeBuildInputs = [ pkgs.git checkBeforePush ]; } ''
          set -u
          export HOME="$TMPDIR"
          export GIT_CONFIG_GLOBAL="$TMPDIR/gitconfig"
          export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
          export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid
          git config --global init.defaultBranch main

          repo="$TMPDIR/repo"
          git init --quiet "$repo"
          printf 'one\n' > "$repo/a.txt"
          git -C "$repo" add a.txt
          git -C "$repo" commit --quiet -m base
          printf 'scratch\n' > "$repo/notes.txt"

          output="$( cd "$repo" && check-before-push 2>&1 )" && status=0 || status=$?

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }
          [ "$status" = 1 ] || fail "expected a refusal (exit 1), got $status:"
          case "$output" in
            *notes.txt*) ;;
            *) fail "the refusal did not name the untracked file:" ;;
          esac

          touch $out
        '';

        # A module that never says what it is for is where a second concern
        # moves in unnoticed, so the sentence is what this holds each module
        # to. Nothing stock asks for it: `missing_docs` fires on public items
        # rather than on modules, and under the `testing` feature the module
        # set itself changes.
        modulesStateTheirConcern = pkgs.writeShellScriptBin "modules-state-their-concern" ''
          set -u

          cd "''${1:-.}" || exit 1

          silent="$(find src -name '*.rs' | sort | while IFS= read -r module; do
            head -n 1 "$module" | grep -q '^//!' || printf '  %s\n' "$module"
          done)"

          if [ -n "$silent" ]; then
            echo "These modules do not open by saying what they are for:"
            printf '%s\n' "$silent"
            echo
            echo "Give each a //! on its first line, naming the one concern it holds."
            exit 1
          fi
        '';

        # Every module in this tree already speaks, so the check above passes
        # whether or not it can still find a silent one. This is what says it
        # can.
        modulesStateTheirConcernTest = pkgs.runCommand "modules-state-their-concern-test"
          { nativeBuildInputs = [ modulesStateTheirConcern ]; } ''
          set -u

          tree="$TMPDIR/tree"
          mkdir -p "$tree/src/layer"
          printf '//! What this one is for.\n\nfn spoken() {}\n' > "$tree/src/spoken.rs"
          printf '/// Not the module, only the item.\nfn quiet() {}\n' > "$tree/src/layer/silent.rs"

          output="$( modules-state-their-concern "$tree" 2>&1 )" && status=0 || status=$?

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }
          [ "$status" = 1 ] || fail "expected a refusal (exit 1), got $status:"
          case "$output" in
            *src/layer/silent.rs*) ;;
            *) fail "the refusal did not name the silent module:" ;;
          esac
          case "$output" in
            *spoken.rs*) fail "it named a module that does say what it is for:" ;;
          esac

          printf '//! What this one is for.\n' > "$tree/src/layer/silent.rs"
          modules-state-their-concern "$tree" ||
            fail "it refused a tree in which every module speaks:"

          touch $out
        '';

        # A test that walks the selection down the screen has to stop
        # somewhere. Stopping when the code under test reports the screen no
        # longer moves is the form a mutation turns into a hang: cargo-mutants
        # scores the hang as a timeout, and on the tally that reads exactly
        # like a mutant which does not terminate in production. `walk::until`
        # in src/view/walk.rs is the walk counted out before it starts, and
        # this is what keeps the other kind from being written beside it —
        # three seats wrote one anyway, each having read the rule.
        #
        # Scoped to a loop that presses a key rather than to every loop a
        # test writes: a `while let` over a worklist, a test double's server
        # thread, a wait on a deadline — each is left alone while it presses
        # nothing, which is what all three do here. One that does press is
        # refused whatever else bounds it. A bound worth sparing it for is a
        # bound this check would have to trust, and it reads indentation
        # rather than meaning; refusing a loop that had a real bound costs a
        # round trip, sparing a walk costs the tally the defect this exists
        # for. A round trip that was the wrong answer comes back here with a
        # fixture. Production is left alone: the rule is about what a mutation
        # does to a test.
        screenWalksAreBounded = pkgs.writeShellScriptBin "screen-walks-are-bounded" ''
          set -u

          cd "''${1:-.}" || exit 1

          # Rust has no lint for this and nothing off the shelf reads block
          # structure, so this does — by indentation, which rustfmt makes
          # reliable and brace-counting is not, because a format string full
          # of braces is what a test is mostly made of.
          reading='
          function loose(text) {
            return text ~ /(^|[^A-Za-z_])(while|loop)([^A-Za-z_]|$)/
          }
          function trim(text) {
            sub(/^ +/, "", text)
            return text
          }
          function blame(at, text,   key) {
            key = FILENAME ":" at
            if (key in reported) {
              return
            }
            reported[key] = 1
            printf "  %s:%d: %s\n", FILENAME, at, trim(text)
          }
          BEGIN { in_test = whole_file }
          {
            line = $0

            if (line ~ /^[ \t]*$/ || line ~ /^ *\/\//) {
              next
            }

            match(line, /^ */)
            ind = RLENGTH

            # rustfmt breaks a long loop header over several lines and leaves
            # the brace on one of its own, so the line that opens a block is
            # not always the line that says what kind of block it is. The
            # header is every line since the last one that ended something.
            if (header == "") {
              header = line
              header_at = FNR
            } else {
              header = header " " trim(line)
            }

            # A line at this indent has closed every block opened inside it.
            for (i = ind; i <= deepest; i++) {
              openers[i] = ""
            }

            if (in_test && !whole_file && line == closer) {
              in_test = 0
            }

            if (after_cfg_test && line ~ /^ *(pub(\([a-z]+\))? )?mod [A-Za-z_0-9]+ \{$/) {
              in_test = 1
              closer = sprintf("%*s}", ind, "")
            }
            after_cfg_test = (line ~ /^ *#\[cfg\(test\)\]$/)

            if (in_test && line ~ /\.apply\(/) {
              if (loose(header)) {
                blame(header_at, header)
              }
              for (i = 0; i < ind; i++) {
                if (loose(openers[i])) {
                  blame(opened_at[i], openers[i])
                }
              }
            }

            if (line ~ /\{[ \t]*$/) {
              openers[ind] = header
              opened_at[ind] = header_at
              if (ind > deepest) {
                deepest = ind
              }
            }

            if (line ~ /[;{},][ \t]*$/) {
              header = ""
            }
          }
          '

          # Under src/ the tests are the `#[cfg(test)]` modules inside the file
          # they cover; everything under tests/ is a test all through.
          loose="$( {
            find src -name '*.rs' | sort | while IFS= read -r module; do
              ${pkgs.gawk}/bin/gawk -v whole_file=0 "$reading" "$module"
            done
            find tests -name '*.rs' | sort | while IFS= read -r module; do
              ${pkgs.gawk}/bin/gawk -v whole_file=1 "$reading" "$module"
            done
          } )"

          if [ -n "$loose" ]; then
            echo "These tests press a key inside a loop nothing counts out:"
            printf '%s\n' "$loose"
            echo
            echo "A loop that ends when the code under test says the screen stopped"
            echo "moving is a loop a mutation can leave running for ever. The test"
            echo "hangs, cargo-mutants scores a timeout, and the tally cannot tell"
            echo "that from a mutant which does not terminate in production."
            echo
            echo "walk::until in src/view/walk.rs presses inside a count taken before"
            echo "the walk starts, and says which row it never reached. Reach for it."
            exit 1
          fi
        '';

        # Every walk in this tree goes through walk::until, so the check above
        # passes whether or not it can still find one that does not. This is
        # what says it can — and what says it stays quiet about the loops a
        # test is entitled to write.
        screenWalksAreBoundedTest = pkgs.runCommand "screen-walks-are-bounded-test"
          { nativeBuildInputs = [ screenWalksAreBounded ]; } ''
          set -u

          tree="$TMPDIR/tree"
          mkdir -p "$tree/src" "$tree/tests"

          # bdi-7ao.52's own regression, and the same walk spelled as a bare
          # loop — which is the spelling a check that only read `while` would
          # hand the next seat.
          {
            echo "//! A walk production code decides the end of."
            echo "#[cfg(test)]"
            echo "mod tests {"
            echo "    #[test]"
            echo "    fn moving_faster_than_herdr_answers_costs_one_read_at_a_time() {"
            echo "        while shown.apply(Action::Move(Motion::NextRow)) {"
            echo "            moved += 1;"
            echo "        }"
            echo "    }"
            echo "    #[test]"
            echo "    fn the_same_walk_as_a_bare_loop() {"
            echo "        loop {"
            echo "            if !shown.apply(Action::Move(Motion::NextRow)) {"
            echo "                break;"
            echo "            }"
            echo "        }"
            echo "    }"
            echo "}"
          } > "$tree/src/loose.rs"

          # The same walk under a header rustfmt broke over three lines, which
          # leaves the brace opening the block on one of its own. A check that
          # read only the line carrying the brace would let this through.
          {
            echo "//! A loop header rustfmt split across lines."
            echo "#[cfg(test)]"
            echo "mod tests {"
            echo "    #[test]"
            echo "    fn a_walk_under_a_header_rustfmt_broke() {"
            echo "        while shown.settling(Action::Refresh)"
            echo "            && shown.forest.selected_line() < wanted"
            echo "        {"
            echo "            shown.apply(Action::Move(Motion::NextRow));"
            echo "        }"
            echo "    }"
            echo "}"
          } > "$tree/src/split.rs"

          # The counted walk, and a worklist loop — which this rule has
          # nothing to say about, because it presses nothing.
          {
            echo "//! Walks counted out before they start."
            echo "#[cfg(test)]"
            echo "mod tests {"
            echo "    #[test]"
            echo "    fn a_counted_walk_reaches_the_last_row() {"
            echo "        for _ in 0..shown.rows() {"
            echo "            shown.apply(Action::Move(Motion::NextRow));"
            echo "        }"
            echo "    }"
            echo "    #[test]"
            echo "    fn a_worklist_the_test_owns_is_not_a_walk() {"
            echo "        while let Some(node) = walking.pop() {"
            echo "            walking.extend(children(node));"
            echo "        }"
            echo "    }"
            echo "}"
          } > "$tree/src/counted.rs"

          # A loop bounded by a deadline of the test's own, which still
          # presses a key. The bound is real and the refusal is deliberate:
          # this is the rule as it is, so that a seat meeting it reads a
          # decision rather than an oversight.
          {
            echo "//! A press inside a bound the code under test cannot lie about."
            echo "#[cfg(test)]"
            echo "mod tests {"
            echo "    #[test]"
            echo "    fn a_press_waiting_on_a_deadline_is_refused_too() {"
            echo "        while Instant::now() < deadline {"
            echo "            shown.apply(Action::Refresh);"
            echo "        }"
            echo "    }"
            echo "}"
          } > "$tree/src/deadline.rs"

          # Production, which this rule says nothing about: a mutation that
          # hangs it is a mutant that hangs in production, which is a finding
          # rather than a false one.
          {
            echo "//! Production, which the rule says nothing about."
            echo "fn settle(&mut self) {"
            echo "    while self.apply(Action::Refresh) {"
            echo "        self.lay_out();"
            echo "    }"
            echo "}"
          } > "$tree/src/production.rs"

          output="$( screen-walks-are-bounded "$tree" 2>&1 )" && status=0 || status=$?

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }
          [ "$status" = 1 ] || fail "expected a refusal (exit 1), got $status:"
          case "$output" in
            *"src/loose.rs:6"*) ;;
            *) fail "the refusal did not name the while walking the screen:" ;;
          esac
          case "$output" in
            *"src/loose.rs:12"*) ;;
            *) fail "the refusal did not name the same walk spelled as a loop:" ;;
          esac
          case "$output" in
            *"src/split.rs:6"*) ;;
            *) fail "the refusal did not reach under a header rustfmt broke:" ;;
          esac
          case "$output" in
            *"src/deadline.rs:6"*) ;;
            *) fail "the refusal did not reach a press inside a deadline:" ;;
          esac
          case "$output" in
            *counted.rs*) fail "it named a walk that is counted out before it starts:" ;;
          esac
          case "$output" in
            *production.rs*) fail "it named production, which this rule is not about:" ;;
          esac

          # A file under tests/ is a test all through, with no #[cfg(test)] to
          # find, so the whole of it is read.
          {
            echo "fn walk_the_screen(forest: &mut Forest) {"
            echo "    while forest.apply(Action::Move(Motion::NextRow)) {}"
            echo "}"
          } > "$tree/tests/integration.rs"

          output="$( screen-walks-are-bounded "$tree" 2>&1 )" && status=0 || status=$?
          [ "$status" = 1 ] || fail "expected a refusal (exit 1), got $status:"
          case "$output" in
            *"tests/integration.rs:2"*) ;;
            *) fail "the refusal did not reach a walk under tests/:" ;;
          esac

          rm "$tree/src/loose.rs" "$tree/src/split.rs"
          rm "$tree/src/deadline.rs" "$tree/tests/integration.rs"
          screen-walks-are-bounded "$tree" ||
            fail "it refused a tree in which every walk is counted out:"

          touch $out
        '';

        # Everything needed to build, test and lint the crate. The tracker
        # client is not here — that is a maintainer's tool, not a
        # contributor's.
        rustTools = [
          pkgs.git
          pkgs.cargo
          pkgs.rustc
          pkgs.rustfmt
          pkgs.clippy
          pkgs.rust-analyzer
          pkgs.watchexec
          rerunBdiOnChange
          checkBeforePush
          readCiVerdict
        ];

        # A check runs against the same source and the same vendored crates as
        # the build, so the two cannot drift apart. It starts from `artifacts`,
        # the dependency build for the profile its command runs at, or from
        # `null` where it compiles nothing — so `cargo fmt` can say a file is
        # misformatted without waiting on a dependency build to say it.
        checkOf = name: artifacts: tools: command:
          beady-eye.overrideAttrs (build: {
            pname = "${build.pname}-${name}";
            cargoArtifacts = artifacts;
            nativeBuildInputs = build.nativeBuildInputs ++ tools;
            buildPhase = ''
              set -o pipefail
              { ${command}
              } 2>&1 | tee "$NIX_BUILD_TOP/check.log"
            '' + pkgs.lib.optionalString (artifacts != null) ''

              # Inheriting a dependency build and then compiling it again is how
              # this arrangement fails: cargo says nothing, the check still
              # passes, and the minutes it was meant to save are gone. Reading
              # the build back is what makes that a failure rather than a
              # slower green.
              rebuilt="$(grep -E '^ +(Compiling|Checking) ' "$NIX_BUILD_TOP/check.log" |
                grep -vE '^ +(Compiling|Checking) ${common.pname} v' || true)"
              if [ -n "$rebuilt" ]; then
                echo "This check compiled dependencies it was handed already built:"
                printf '%s\n' "$rebuilt"
                echo
                echo "A dependency build is only reused at the cargo profile it was"
                echo "built at, so the command above and the artifacts it inherits"
                echo "have to agree on one. See artifactsFor."
                exit 1
              fi
            '';
            doCheck = false;
            installPhase = "touch $out";
            dontFixup = true;
          });
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = rustTools;

          shellHook = ''
            # The banner is diagnostic, so it goes where nix puts its own
            # diagnostics. On stdout it corrupts every `nix develop -c … --json`
            # a caller pipes into a parser.
            echo "👁  beady-eye Development Shell" >&2
          '';
        };

        # The default shell plus `bd`, the client for the maintainers' issue
        # tracker. That tracker is not part of this repository — contributors
        # file GitHub issues instead, see CLAUDE.md — so `bd` and everything
        # that points it at a tracker live here rather than in `default`, and
        # entering this shell is opt-in. Select it locally with an untracked
        # `.envrc.local` containing `devshell=maintainer`.
        devShells.maintainer = pkgs.mkShell {
          buildInputs = rustTools ++ [ beads.packages.${system}.bd ];

          shellHook = ''
            # Neither the tracker's coordinates nor its password are checked
            # in any more, so a worktree has none of its own to find. Both live
            # in the main checkout; resolve it once and read both from there.
            main_checkout="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"

            # Point every worktree at the one .beads/ — the same thing bd's own
            # worktree support does with a redirect file — rather than let each
            # keep a copy that can drift. Setting this at all is also what
            # stops a bare bd from inheriting another project's BEADS_DIR.
            export BEADS_DIR="$main_checkout/.beads"

            # Disable bd's smart remote-migrate gate. It is the only verdict
            # that can permit an in-place schema migration of a shared server,
            # and nothing here should ever migrate one.
            export BD_SMART_GATE=0

            echo "👁  beady-eye maintainer shell" >&2
            # Print where bd resolved from, not just what it claims to be — a
            # version alone cannot distinguish this shell's bd from PATH's.
            echo "beads: $(bd --version) ($(command -v bd))" >&2

            if [ -f "$main_checkout/.env.local" ]; then
              set -a
              source "$main_checkout/.env.local"
              set +a
              echo "✅ Loaded environment from .env.local" >&2
            else
              echo "⚠️  no .env.local — bd cannot authenticate to the tracker" >&2
            fi
          '';
        };

        packages.default = beady-eye;
        packages.beady-eye = beady-eye;

        # `nix flake check` is the whole of CI. Anything CI should run belongs
        # here, not in the workflow that calls it.
        checks = {
          build-and-test = beady-eye;
          check-before-push = checkBeforePushTest;
          clippy = checkOf "clippy" artifacts.dev [ pkgs.clippy ] "cargo clippy --all-targets -- -D warnings";

          # A second invocation rather than a flag on the one above, because
          # `--all-targets` is what defeats it: building the test targets pulls
          # in the dev-dependencies, which turns `testing` on, which makes the
          # library's modules `pub` again and switches `dead_code` off. Only a
          # build without them sees the narrow surface. See src/lib.rs.
          dead-code = checkOf "dead-code" artifacts.dev [ pkgs.clippy ] "cargo clippy -- -D warnings";
          fmt = checkOf "fmt" null [ pkgs.rustfmt ] "cargo fmt --check";
          module-concerns = checkOf "module-concerns" null [ modulesStateTheirConcern ]
            "modules-state-their-concern";
          module-concerns-test = modulesStateTheirConcernTest;
          screen-walks = checkOf "screen-walks" null [ screenWalksAreBounded ]
            "screen-walks-are-bounded";
          screen-walks-test = screenWalksAreBoundedTest;

          # cargo publish uploads only what Cargo.toml's include list selects,
          # and builds that tarball rather than the working tree. A crate that
          # compiles here and not from the tarball is otherwise found by
          # whoever depends on it first, and the version cannot be withdrawn.
          #
          # The verify build catches anything the compiler would miss having.
          # It cannot catch a dropped library or binary: cargo drops those with
          # a warning and an exit code of zero, then verifies a tarball with
          # nothing in it. The tests are left out on purpose, so only the two
          # targets the crate exists to ship are fatal here.
          package = checkOf "package" artifacts.dev [ ] ''
            set -o pipefail
            cargo package --offline --locked 2>&1 | tee package.log
            ! grep -qE "ignoring (library|binary) .* is not included" package.log
          '';
        };
      }
    ) // {
      # Overlays carry no system, so this sits outside eachDefaultSystem. A
      # consumer adds it to nixpkgs.overlays and reaches pkgs.beady-eye.
      overlays.default = final: _prev: {
        beady-eye = beadyEyeFor final;
      };
    };
}

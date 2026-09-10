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

      # Every check is built from a source tree, and nix hashes the whole of
      # the one it is handed. So a file in there that no check reads still
      # gives every check a derivation nothing has built before, and the run
      # rebuilds all of them for a verdict the tree already has: a pull request
      # changing only README.md paid for the entire suite.
      #
      # An allowlist, for the reason Cargo.toml's `include` is one — a file
      # that arrives later is out until somebody says so. Out is also the safe
      # direction: a check that needed the file fails saying it is missing,
      # where an exclude list that forgot it goes on quietly rebuilding.
      sourceOf = paths: nixpkgs.lib.fileset.toSource {
        root = ./.;
        fileset = nixpkgs.lib.fileset.unions paths;
      };

      # What the compiler, the tests and the tree scans read, and nothing else.
      # Every `include_str!` in the crate points inside tests/fixtures/, and
      # the two tests that start from CARGO_MANIFEST_DIR walk into src/ and
      # tests/shims/.
      source = sourceOf [ ./Cargo.toml ./Cargo.lock ./src ./tests ];

      # The crate names the version once. The release tag is cut from it and
      # this output is compared against it before anything is published, so a
      # crate on crates.io always has a flake output built from the same source
      # at the same version.
      common = {
        pname = cargoToml.package.name;
        version = cargoToml.package.version;
        src = source;
      };

      # crane strips the crate's own code out of the dependency build but hands
      # the manifests through, so the crate's real version would reach that
      # build in the files and key the graph on the one thing a release changes.
      versionForDeps = "0.0.0";

      # Cargo states a package's name and version on consecutive lines, in the
      # manifest and in the lock entry alike, and that pair is the only place
      # either file names this crate's own version. So a manifest that stops
      # spelling it that way throws: a pin that silently matches nothing reads
      # exactly like one that worked.
      pinnedManifest = file:
        let
          text = builtins.readFile file;
          declaration = version: builtins.concatStringsSep "\n" [
            ''name = "${cargoToml.package.name}"''
            ''version = "${version}"''
          ];
          pinned = builtins.replaceStrings
            [ (declaration cargoToml.package.version) ]
            [ (declaration versionForDeps) ]
            text;
        in
        if pinned == text
        then throw "${builtins.baseNameOf file} no longer states this crate's name and version on consecutive lines, so the dependency graph cannot be pinned off the version"
        else pinned;

      # The dependency graph, compiled on its own and keyed on Cargo.lock rather
      # than on the source, so one compilation serves every later check — which
      # is the whole point, because compiling it is most of what this project
      # waits for. In CI the Actions cache is scoped per branch, so `main` seeds
      # a graph every pull request restores, while a pull request seeds only
      # itself.
      #
      # Naming the source tree in this derivation would put every later edit to
      # it back into the dependency build, so it gets the manifests alone —
      # which is what crane reduces a tree to anyway, plus dummies of the
      # targets Cargo.toml declares.
      #
      # Two of them, because a check reuses artifacts only at the profile it was
      # built at, and the checks are not all at one profile: the package builds
      # and tests at release, while clippy, the dead-code pass and `cargo
      # package`'s verify build all run at dev. Building both is what leaves
      # every check's command exactly as it was.
      artifactsFor = pkgs:
        let
          craneLib = crane.mkLib pkgs;
          depsCommon = common // {
            version = versionForDeps;
            src = pkgs.runCommand "beady-eye-deps-source" { } ''
              mkdir -p $out
              cp ${pkgs.writeText "Cargo.toml" (pinnedManifest ./Cargo.toml)} $out/Cargo.toml
              cp ${pkgs.writeText "Cargo.lock" (pinnedManifest ./Cargo.lock)} $out/Cargo.lock
            '';
          };
        in
        {
          release = craneLib.buildDepsOnly depsCommon;
          dev = craneLib.buildDepsOnly (depsCommon // {
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

        # What `cargo package` is handed. It builds the tarball Cargo.toml's
        # `include` list selects, and refuses a `readme` it cannot find — but
        # all it does with README.md and LICENSE is copy them, and no part of
        # the check reads a word of either. So it gets a stand-in for both, and
        # rewriting the README stops being a reason to build anything.
        #
        # `pathExists` rather than the paths themselves: naming a path is what
        # would put the file's contents back into the derivation. Deleting one
        # is still refused here rather than at publishing time.
        publishedSource =
          let
            copied = [ "README.md" "LICENSE" ];
            absent = builtins.filter (f: !builtins.pathExists (./. + "/${f}")) copied;
            stoodInFor = pkgs.runCommand "published-source" { } ''
              cp -r ${sourceOf [ ./Cargo.toml ./Cargo.lock ./src ]} "$out"
              chmod -R u+w "$out"
              for file in ${builtins.concatStringsSep " " copied}; do
                echo "Stood in for; see publishedSource in flake.nix." > "$out/$file"
              done
            '';
          in
          nixpkgs.lib.throwIf (absent != [ ])
            ("Cargo.toml's include list names ${builtins.concatStringsSep " and " absent}, "
              + "which cargo package needs and this tree has not got.")
            stoodInFor;

        # Both properties above are silent when they break. Put a documentation
        # file back into either source and every check still passes, on the
        # same command, with the same output — only having built what it was
        # handed already built. Nothing in a green run says which of the two
        # happened, so they have to be stated somewhere they can fail.
        documentationIsNotSource =
          pkgs.runCommand "documentation-is-not-source" { } ''
            carried="$(find ${source} \( -name '*.md' -o -name docs \) )"
            if [ -n "$carried" ]; then
              echo "The source the checks are built from carries documentation:"
              printf '%s\n' "$carried"
              echo
              echo "Every check hashes the whole of that tree, so a change to any"
              echo "of these rebuilds all of them for a verdict the tree already"
              echo "has. Take it out of \`source\` in flake.nix — the file stays in"
              echo "the repository, it just stops being something a check reads."
              exit 1
            fi

            if ! grep -q 'Stood in for' ${publishedSource}/README.md; then
              echo "The package check has been handed the repository's own README.md."
              echo
              echo "Its contents then decide that check's derivation, so every"
              echo "rewrite of it builds the crate again to learn what the last"
              echo "one already proved. See publishedSource in flake.nix."
              exit 1
            fi

            touch $out
          '';

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
            -- 'cargo build --release --quiet && exec target/release/bdi'
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

        # GitHub records `cancelled` for a job its own `timeout-minutes` killed,
        # and there is nothing else to read it off: the run, the job and the log
        # all say exactly what they say for a run somebody cancelled by hand.
        # The check-run annotation is the only place the difference is written
        # down, so this is what tells the two apart. It prints the limit and
        # exits non-zero when the annotations name none.
        limitTheJobExceeded = ''
          limit_the_job_exceeded() {
            ${pkgs.gnugrep}/bin/grep -m1 'exceeded the maximum execution time'
          }
        '';

        # The sentence above is GitHub's, not ours, so nothing in this tree
        # fails when it changes. This is what says the pattern still matches the
        # one that was measured, against both annotation sets from the push that
        # measured it.
        limitTheJobExceededTest = pkgs.runCommand "limit-the-job-exceeded-test" { } ''
          set -u
          ${limitTheJobExceeded}

          timed_out='The job has exceeded the maximum execution time of 1m0s
          The operation was canceled.'
          by_hand='The run was canceled by @GraemeF.
          The operation was canceled.'

          found="$(printf '%s\n' "$timed_out" | limit_the_job_exceeded)" || found=""
          case "$found" in
            *'maximum execution time of 1m0s'*) ;;
            *)
              echo "FAIL: the timeout annotation was not recognised, or its limit"
              echo "was not what came back. Got: '$found'"
              exit 1
              ;;
          esac

          if printf '%s\n' "$by_hand" | limit_the_job_exceeded; then
            echo "FAIL: a run cancelled by hand was read as one that timed out."
            exit 1
          fi

          touch $out
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

          ${limitTheJobExceeded}

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

          A cancelled run is neither green nor red: the commit has no verdict.
          On a pull request it is the branch moving: a push cancels the run on
          the head it replaced, and the verdict lives on the new head, which
          this names.

          A run cancelled with its head still in place is one of two things,
          and GitHub records them identically — same run conclusion, same job
          conclusion, and a log ending "The operation was canceled." either
          way. Only the check-run annotation says which, so this reads that:

            ran out of time     ci.yml caps the job at timeout-minutes, and a
                                job that reaches the cap is killed and recorded
                                cancelled. Starting it again reproduces
                                whatever hung and spends the cap over again.
            cancelled by hand   Nothing else did it, so it wants starting
                                again.
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
              --json databaseId,headSha,headBranch,status,conclusion,workflowName,url,createdAt
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
              branch="$(printf '%s' "$latest" |
                $jq -r 'first(.[] | select(.conclusion == "cancelled")) | .headBranch')"
              moved="$($gh api "repos/{owner}/{repo}/commits/$subject/pulls" |
                $jq -r --arg sha "$subject" --arg branch "$branch" \
                  'first(.[] | select(.head.ref == $branch and .head.sha != $sha)) |
                   "\(.head.sha) \(.html_url)"')"
              if [ -n "$moved" ]; then
                echo "NO VERDICT — the run for $subject was cancelled because $branch"
                echo "moved on. Its verdict lives on the head that replaced it:"
                echo "  ''${moved%% *}"
                echo "  ''${moved#* }"
                exit 1
              fi

              # Nobody cancelled a run its own timeout killed, so "start it
              # again" is the one instruction that cannot help: the rerun hangs
              # the same way and spends the timeout over again.
              #
              # Every cancelled job of every cancelled run is asked, because one
              # timed-out job among several cancelled ones is still a commit
              # whose rerun will hang, and asking only the first reports the
              # timeout or misses it depending on which order gh answered in.
              #
              # A gh that fails answers with nothing, which is what a run
              # cancelled by hand also answers with, so a question that could
              # not be asked would come out as "start it again". Each call says
              # so instead.
              github_would_not_say() {
                echo "NO VERDICT — a run for $subject was cancelled, and GitHub would"
                echo "not say whether its own timeout did it. That reads exactly like a"
                echo "cancellation by hand, so this is not telling you to start it again."
                echo "Ask again."
                exit 1
              }

              limit=""
              for run in $(printf '%s' "$latest" |
                  $jq -r '.[] | select(.conclusion == "cancelled") | .databaseId'); do
                checks="$($gh api "repos/{owner}/{repo}/actions/runs/$run/jobs" \
                    --jq '.jobs[] | select(.conclusion == "cancelled") | .check_run_url')" ||
                  github_would_not_say
                for check in $checks; do
                  messages="$($gh api "$check/annotations" --jq '.[].message')" ||
                    github_would_not_say
                  if [ -z "$limit" ]; then
                    limit="$(printf '%s\n' "$messages" | limit_the_job_exceeded)" || limit=""
                  fi
                done
              done

              if [ -n "$limit" ]; then
                echo "NO VERDICT — a run for $subject ran itself out of time:"
                echo "  $limit"
                echo "Nothing cancelled it and starting it again reproduces whatever"
                echo "hung. The log names it: libtest prints \"has been running for"
                echo "over 60 seconds\" for a test that never returned, and the job"
                echo "log says only that the operation was canceled."
                exit 1
              fi

              echo "NO VERDICT — a run for $subject was cancelled with $branch still"
              echo "heading there, so nothing superseded it. Start it again."
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

        # `cargo-mutants --in-diff` exits 0 on a diff it cannot score, and that
        # is the same 0 a run which caught every mutant exits. Two runs reach
        # it, both measured at 27.1.0: a diff holding no mutable production
        # line prints "No mutants to filter" and writes no mutants.out at all,
        # and a diff whose every mutant is unviable prints "No mutants were
        # viable" and writes one whose every scored row is zero. Neither tested
        # the change. This is the count that tells them from a run that did,
        # and it refuses them, so the reader is not the guard.
        #
        # It reads the run's own output directory rather than the repository's,
        # because a vacuous run leaves an earlier mutants.out exactly where it
        # stood — same inode, same counts, and no mutants.out.old beside it —
        # so the tree's copy answers for whichever run last wrote one.
        refuseARunThatScoredNothing = ''
          refuse_a_run_that_scored_nothing() {
            jq=${pkgs.jq}/bin/jq
            outcomes="$1/mutants.out/outcomes.json"

            if [ ! -f "$outcomes" ]; then
              echo "Nothing was scored: the run wrote no $outcomes."
              echo "cargo-mutants writes none when the diff holds no mutable"
              echo "production line, so this says nothing about the change."
              exit 1
            fi

            scored="$($jq '.caught + .missed + .timeout' "$outcomes")"
            if [ "$scored" = 0 ]; then
              echo "Nothing was scored: $($jq -r '"\(.total_mutants) generated, \(.unviable) unviable"' "$outcomes")."
              echo "An unviable mutant does not compile, so it tests nothing,"
              echo "and this says nothing about the change."
              exit 1
            fi

            echo "Scored $scored mutants."
          }
        '';

        # Both runs above pass anybody reading the exit code, so this guard is
        # worth having only if it fires. These are those two runs, reduced to
        # the tally the count is read off. The third row is the control: a
        # guard that refused everything would pass the first two, and nothing
        # in them could tell it apart from one that works.
        refuseARunThatScoredNothingTest =
          pkgs.runCommand "refuse-a-run-that-scored-nothing-test" { } ''
          set -u
          ${refuseARunThatScoredNothing}

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }

          tally() {
            mkdir -p "$1/mutants.out"
            printf '%s\n' "$2" > "$1/mutants.out/outcomes.json"
          }

          # A diff with no mutable production line. cargo-mutants writes
          # nothing, so the directory it was pointed at stays empty.
          mkdir -p "$TMPDIR/nothing"
          output="$( refuse_a_run_that_scored_nothing "$TMPDIR/nothing" 2>&1 )" &&
            fail "it accepted a run that wrote no tally at all:"
          case "$output" in
            *"wrote no"*) ;;
            *) fail "the refusal did not say the run wrote no tally:" ;;
          esac

          # A diff whose every mutant is unviable. Here there is a tally, and
          # every row of it that means a mutant was tested is zero.
          tally "$TMPDIR/unviable" \
            '{"total_mutants":1,"caught":0,"missed":0,"timeout":0,"unviable":1}'
          output="$( refuse_a_run_that_scored_nothing "$TMPDIR/unviable" 2>&1 )" &&
            fail "it accepted a run whose every mutant was unviable:"
          case "$output" in
            *"1 generated, 1 unviable"*) ;;
            *) fail "the refusal did not say what the run generated:" ;;
          esac

          # A run that tested the change.
          tally "$TMPDIR/scored" \
            '{"total_mutants":5,"caught":4,"missed":1,"timeout":0,"unviable":0}'
          output="$( refuse_a_run_that_scored_nothing "$TMPDIR/scored" 2>&1 )" ||
            fail "it refused a run that scored five mutants:"
          case "$output" in
            *"Scored 5 mutants"*) ;;
            *) fail "it accepted the run without saying what it scored:" ;;
          esac

          touch $out
        '';

        # cargo-mutants copies the working tree and tests that, so the filter
        # has to describe the working tree too. Taken from HEAD it would name
        # committed lines while the uncommitted edits beside them went
        # unmutated, and the run would report a tally for a revision nobody
        # was testing.
        #
        # The merge base rather than the ref itself, which is what `...` means
        # and is the whole of why that form is written everywhere here: a diff
        # against the ref carries in reverse whatever landed on it while you
        # worked, and cargo-mutants scores those lines on your tree as your
        # tally.
        scopeToTheChange = ''
          scope_to_the_change() {
            git=${pkgs.git}/bin/git

            # An untracked file that is not ignored is copied into the tree
            # cargo-mutants tests, and `git diff` cannot see one at all — so a
            # new module's lines would be in the source under test and outside
            # the filter, and nothing would mutate any of them. That is a full
            # tally saying nothing about the file the change is about.
            loose="$($git ls-files --others --exclude-standard -- '*.rs')"
            if [ -n "$loose" ]; then
              echo "These Rust files are untracked, so they are in the tree"
              echo "cargo-mutants tests and out of the diff that scopes it:"
              printf '%s\n' "$loose"
              echo
              echo "Stage them (git add -N is enough) and run this again."
              exit 1
            fi

            base="$($git merge-base "$1" HEAD)" || exit 1
            echo "Scoping to the change since $($git rev-parse --short "$base"), the merge base with $1."
            $git diff "$base" > "$2" || exit 1
          }
        '';

        # Both properties above are silent when they break: a filter naming the
        # wrong revision still produces a diff, still generates mutants, and
        # still prints a tally. These are the two lines that must be in it and
        # the one that must not.
        scopeToTheChangeTest = pkgs.runCommand "scope-to-the-change-test"
          { nativeBuildInputs = [ pkgs.git ]; } ''
          set -u
          ${scopeToTheChange}

          export HOME="$TMPDIR"
          export GIT_CONFIG_GLOBAL="$TMPDIR/gitconfig"
          export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
          export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid
          git config --global init.defaultBranch main

          repo="$TMPDIR/repo"
          git init --quiet "$repo"
          printf 'base\n' > "$repo/a.txt"
          git -C "$repo" add a.txt
          git -C "$repo" commit --quiet -m base

          git -C "$repo" checkout --quiet -b work
          printf 'a line I committed\n' >> "$repo/a.txt"
          git -C "$repo" commit --quiet -am mine

          # The base moves under the branch, as it does whenever somebody
          # else merges while you work.
          git -C "$repo" checkout --quiet main
          printf 'a line somebody else landed\n' > "$repo/b.txt"
          git -C "$repo" add b.txt
          git -C "$repo" commit --quiet -m theirs
          git -C "$repo" checkout --quiet work

          printf 'a line I have not committed\n' >> "$repo/a.txt"

          ( cd "$repo" && scope_to_the_change main "$TMPDIR/change.diff" )
          scoped="$(cat "$TMPDIR/change.diff")"

          fail() { echo "FAIL: $1"; printf '%s\n' "$scoped"; exit 1; }

          case "$scoped" in
            *"a line I committed"*) ;;
            *) fail "the change I committed was left out of the diff:" ;;
          esac

          # cargo-mutants tests the working tree, so a filter that stops at
          # HEAD leaves this line in the source under test and out of the
          # filter, and nothing mutates it.
          case "$scoped" in
            *"a line I have not committed"*) ;;
            *) fail "the change I had not committed was left out of the diff:" ;;
          esac

          # Taken against the ref rather than the merge base, this arrives in
          # reverse and is scored on your tree as your tally.
          case "$scoped" in
            *"a line somebody else landed"*)
              fail "what landed on the base while I worked is in my diff:" ;;
          esac

          # A new module arrives untracked, and cargo-mutants tests it while
          # git diff cannot see it.
          mkdir -p "$repo/src"
          printf 'pub fn fresh() -> bool { true }\n' > "$repo/src/loose.rs"
          scoped="$( cd "$repo" && scope_to_the_change main "$TMPDIR/change.diff" 2>&1 )" &&
            fail "it scoped a run whose new module was untracked:"
          case "$scoped" in
            *src/loose.rs*) ;;
            *) fail "the refusal did not name the untracked module:" ;;
          esac

          touch $out
        '';


        # The wrapper minted its run directory silently and named it only on
        # the way out, so a seat that backgrounded the run went looking for it
        # — and on a box running two seats, "the newest
        # /tmp/mutation-test-this-change.XXXXXX" is the other seat's run. That
        # happened on 2026-09-03: a seat read a 198-mutant tally belonging to
        # another, and escaped only because the two diffs were flagrantly
        # disjoint. Nothing in the reading disagreed with anything else in it,
        # because the summary line and the file list are both correct about
        # whichever run they come from.
        #
        # So the name carries what tells runs apart, and minting says it
        # aloud. cargo-mutants already names its build tree after the working
        # tree it copied, which under this fleet's worktrees is the seat — the
        # seat was in hand at the moment the run directory was minted and went
        # unused. The head sha is the other half, and it is the half a seat
        # cannot recover by reading the run: your own successive runs share a
        # tree, and after a merge that adds no new file two of them hold
        # byte-identical change.diffs, so the diff separates you from a peer
        # and cannot separate you from yourself.
        #
        # Minting and saying are one function because they were two moments,
        # and the gap between them is the whole of the defect. The name is
        # what survives a seat that backgrounds the run behind `| tail`: the
        # line is the convenience, the directory's own name is the guarantee.
        nameTheRunsDirectory = ''
          name_the_runs_directory() {
            git=${pkgs.git}/bin/git

            # Named under /tmp rather than TMPDIR: a seat whose dev shell is
            # too old for this command reaches a current one with `nix develop
            # <ref> --command`, and that shell's TMPDIR is torn down when the
            # command returns, taking the artefacts this points at with it.
            tree="$(printf '%s' "$(basename "$($git rev-parse --show-toplevel)")" |
                      tr -c 'A-Za-z0-9._-' '-')"
            run="$(mktemp -d "/tmp/mutation-test-this-change-$tree-$($git rev-parse --short HEAD).XXXXXX")"

            echo "This run's output directory is $run."
          }
        '';

        # Two runs told apart by their timestamps is what put another seat's
        # tally in front of a reader, so these are the two populations that
        # reading confused — a peer's run and your own earlier one — with the
        # control that says the comparison can come out equal.
        nameTheRunsDirectoryTest = pkgs.runCommand "name-the-runs-directory-test"
          { nativeBuildInputs = [ pkgs.git ]; } ''
          set -u
          ${nameTheRunsDirectory}

          export HOME="$TMPDIR"
          export GIT_CONFIG_GLOBAL="$TMPDIR/gitconfig"
          export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
          export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid
          git config --global init.defaultBranch main

          fail() { echo "FAIL: $1"; shift; printf '%s\n' "$@"; exit 1; }

          seat() {
            git init --quiet "$TMPDIR/$1"
            printf 'base\n' > "$TMPDIR/$1/a.txt"
            git -C "$TMPDIR/$1" add a.txt
            git -C "$TMPDIR/$1" commit --quiet -m base
          }

          # mktemp's suffix differs whatever else does, so every comparison
          # below is on the name without it.
          stem() { printf '%s' "''${1%.*}"; }

          seat bdi-aid
          seat bdi-yjsj

          cd "$TMPDIR/bdi-aid"
          name_the_runs_directory > "$TMPDIR/said"
          mine="$run"
          said="$(cat "$TMPDIR/said")"

          [ -d "$mine" ] || fail "it named a directory it had not minted:" "$mine"

          # The defect itself: the directory existed and the seat was not told.
          case "$said" in
            *"$mine"*) ;;
            *) fail "minting the directory did not say where it is:" "$said" ;;
          esac

          # A peer's run, minted in the same second as yours.
          cd "$TMPDIR/bdi-yjsj"
          name_the_runs_directory > /dev/null
          theirs="$run"
          [ "$(stem "$(basename "$mine")")" != "$(stem "$(basename "$theirs")")" ] ||
            fail "two seats' runs are named alike:" "$mine" "$theirs"

          # Your own earlier run, on the tree you had before you merged.
          cd "$TMPDIR/bdi-aid"
          printf 'what I merged\n' >> a.txt
          git commit --quiet -am merged
          name_the_runs_directory > /dev/null
          [ "$(stem "$(basename "$mine")")" != "$(stem "$(basename "$run")")" ] ||
            fail "one seat's runs on two trees are named alike:" "$mine" "$run"

          # The control. Nothing above separates a name that carries the seat
          # and the head from one that is simply random per call, because
          # mktemp makes every name unique whatever the stem holds. Two runs
          # of one seat on one tree must therefore land on the same stem.
          earlier="$run"
          name_the_runs_directory > /dev/null
          [ "$(stem "$(basename "$earlier")")" = "$(stem "$(basename "$run")")" ] ||
            fail "the name describes the moment rather than the run:" "$earlier" "$run"

          touch $out
        '';

        # A mutant that does not terminate ends a run in one of two ways, and
        # only one of them leaves a verdict behind, so the timeout beside this
        # and the cap here are both needed.
        #
        # The timeout is what produces the honest word: cargo-mutants kills the
        # test that exceeds it, records TIMEOUT, and goes on to the next
        # mutant. The cap is what keeps the machine while that clock runs, and
        # it cannot stand in for the timeout. Measured at 27.1.0 on a crate
        # whose `*` -> `/` mutant loops allocating: under a memory scope alone
        # that mutant is contained, because the kernel kills the test binary
        # two levels below cargo-mutants and the run survives it — but the run
        # reads `7 caught`, since `cargo test` exits 101 and nothing tells that
        # from a test which failed honestly. A cap on its own turns the
        # pathology into a clean sheet. The same crate under `-t 5` reads
        # `6 caught, 1 timeouts`.
        #
        # 8G is an honest clean build of every test binary plus the whole suite
        # measured at 3.2G, with room over it. MemorySwapMax=0 so the bound is
        # on memory rather than on a machine that is alive and paging.
        # OOMPolicy=continue so the kill takes the process that asked rather
        # than everything in the scope, which is what lets the run reach a
        # verdict on the mutants after it.
        #
        # systemd-run is resolved from PATH rather than pinned, because what it
        # talks to is the machine's own service manager and nothing in the
        # closure. Naming pkgs.systemd would stop this flake evaluating on
        # darwin to buy that, and a mac cannot run this command anyway.
        boundTheMachine = ''
          bound_the_machine() {
            if [ -n "''${MUTATION_TEST_BOUND-}" ]; then
              return 0
            fi

            if ! command -v systemd-run > /dev/null 2>&1; then
              echo "There is no systemd-run here, so this run cannot be given a"
              echo "memory bound, and an unbounded one is how mutation testing"
              echo "takes a machine out of memory rather than timing out."
              echo "Refusing to start."
              echo
              echo "A machine with no user service manager cannot run this"
              echo "command. Score the change on one that has, rather than"
              echo "reaching past this for cargo-mutants itself."
              exit 1
            fi

            MUTATION_TEST_BOUND=1
            export MUTATION_TEST_BOUND

            echo "Bounding this run to ''${MUTATION_TEST_MEMORY_MAX:-8G} of memory."
            exec systemd-run --user --scope --quiet \
              -p MemoryMax="''${MUTATION_TEST_MEMORY_MAX:-8G}" \
              -p MemorySwapMax=0 \
              -p OOMPolicy=continue \
              -- "$@"
          }
        '';

        # Two ways this guard passes a reader while protecting nothing. It can
        # hand a working command to a machine it cannot bound, which is the
        # state the whole thing is here to end. Or it can re-enter itself for
        # ever, since what it runs under the scope is the command that
        # establishes the scope.
        #
        # The scope itself is not made here: a nix build has no user service
        # manager, which is exactly the machine the refusal is about. What
        # stands in for one is a systemd-run that records the properties it was
        # asked for and then runs what followed the `--`, so the run reaching
        # the body at all is the re-entry terminating.
        boundTheMachineTest = pkgs.runCommand "bound-the-machine-test" { } ''
          set -u

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }

          mkdir -p "$TMPDIR/bin"
          {
            echo '#!${pkgs.bash}/bin/bash'
            echo 'printf "%s\n" "$@" > "$TMPDIR/asked"'
            echo 'while [ "$1" != "--" ]; do shift; done'
            echo 'shift'
            echo 'exec "$@"'
          } > "$TMPDIR/bin/systemd-run"
          chmod +x "$TMPDIR/bin/systemd-run"

          {
            echo '#!${pkgs.bash}/bin/bash'
            echo 'set -u'
            cat ${pkgs.writeText "bound-the-machine.sh" boundTheMachine}
            echo 'bound_the_machine "$0" "$@"'
            echo 'echo "the body ran"'
          } > "$TMPDIR/command"
          chmod +x "$TMPDIR/command"

          export PATH="$TMPDIR/bin:$PATH"
          output="$( "$TMPDIR/command" 2>&1 )" ||
            fail "it refused a machine that could bound the run:"

          # Once, rather than once per scope for ever.
          ran="$(printf '%s\n' "$output" | ${pkgs.gnugrep}/bin/grep -c "the body ran")"
          [ "$ran" = 1 ] ||
            fail "the body ran $ran times, so the re-entry does not stop:"

          asked="$(cat "$TMPDIR/asked")"
          for property in MemoryMax=8G MemorySwapMax=0 OOMPolicy=continue --user --scope; do
            case "$asked" in
              *"$property"*) ;;
              *) output="$asked"; fail "the scope was asked for no $property:" ;;
            esac
          done

          # The control: a machine with no user service manager. Without this,
          # a guard that never bounded anything and simply ran the command
          # would pass everything above.
          rm "$TMPDIR/bin/systemd-run"
          output="$( "$TMPDIR/command" 2>&1 )" &&
            fail "it ran unbounded where nothing could bound it:"
          case "$output" in
            *"the body ran"*) fail "it refused and ran the command anyway:" ;;
          esac
          case "$output" in
            *"no systemd-run"*) ;;
            *) fail "the refusal did not say what it could not find:" ;;
          esac
          case "$output" in
            *"reaching past this for cargo-mutants"*) ;;
            *) fail "the refusal did not say what to do instead:" ;;
          esac

          touch $out
        '';

        # The count above only reaches a seat that runs it, so this is the
        # command to run in place of cargo-mutants: it scopes the run to the
        # change, gives it an output directory of its own and says which
        # before the run starts, and refuses one that scored nothing.
        # Everything else passes through.
        #
        # The fetch is the one read-ci-verdict needs, for the same reason.
        # Three dots take the merge base, so a stale origin/main takes an older
        # one and hands cargo-mutants whatever landed on the base while you
        # worked — scored on your tree, as your tally. A fetch can lose a race
        # for a ref another worktree is moving, which is no reason to abandon
        # the run, so this names the commit it resolved rather than insisting
        # on one.
        #
        # The test timeout is fixed rather than derived. cargo-mutants derives
        # one at five times the baseline test run, and this suite's baseline is
        # 84 seconds because the pty tests spend it waiting on a terminal
        # rather than computing — so the derived number came out at 424
        # seconds, which is slack for a loaded machine read as a claim that a
        # test might honestly need that long. A mutant allocating at the rate
        # the words_of one did reaches tens of gigabytes inside it.
        #
        # 180 is that 84 with room for a machine running three seats. A
        # legitimate test that times out here is a test that has got slower,
        # and the answer is to find out which one rather than to raise this.
        # MUTATION_TEST_TIMEOUT raises it for one run while you do — it is an
        # environment variable rather than a flag because cargo-mutants
        # refuses `--timeout` twice, so a caller's own would collide with this
        # one rather than override it.
        mutationTestTimeout = "180";

        mutationTestThisChange =
          pkgs.writeShellScriptBin "mutation-test-this-change" ''
          set -u

          git=${pkgs.git}/bin/git

          ${boundTheMachine}
          ${refuseARunThatScoredNothing}
          ${scopeToTheChange}
          ${nameTheRunsDirectory}

          bound_the_machine "$0" "$@"

          cd "$($git rev-parse --show-toplevel)" || exit 1

          $git fetch --quiet origin ||
            echo "Could not fetch; origin/main is as you left it."

          name_the_runs_directory
          scope_to_the_change origin/main "$run/change.diff"

          timeout="''${MUTATION_TEST_TIMEOUT:-${mutationTestTimeout}}"
          echo "Timing out any test that runs longer than ''${timeout}s."

          ${pkgs.cargo-mutants}/bin/cargo-mutants mutants \
            --timeout "$timeout" \
            --in-diff "$run/change.diff" --output "$run" "$@"
          status=$?

          echo
          refuse_a_run_that_scored_nothing "$run"
          echo "Its artefacts are under $run/mutants.out."

          exit $status
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

        # A colour or a weight spelled at the site that draws it is how this
        # view reached three palettes and six inline greys: each literal is
        # convenient where it is written, and the cost lands on whoever next
        # has to change what a grey means. src/view/palette.rs is the one
        # list, and this is what holds every other module to it.
        #
        # src/view/sgr.rs is exempt. It folds a pane's own escapes back into
        # styles, so the colours it names are the pane's rather than choices
        # of bdi's, and no palette can hold them.
        #
        # Test code is exempt too, and for the opposite reason: a test that
        # pins bd's own 24-bit value, or the escape a pane sent, is the only
        # thing holding the palette to what it is quoting. Route those through
        # the palette and they assert it against itself.
        coloursComeFromThePalette = pkgs.writeShellScriptBin "colours-come-from-the-palette" ''
          set -u

          cd "''${1:-.}" || exit 1

          # A test module is `#[cfg(test)]` at column 0 above an item that
          # opens a block, and it ends at the next line closing one at column
          # 0. Two things sit between the attribute and that item: further
          # attributes, and a bare `mod foo;`, which opens nothing.
          #
          # The two miss in opposite directions. Reading a bare declaration as
          # a block skips every production line after it and says nothing —
          # the failure this check exists to prevent, inside the check.
          # Stopping at an intervening attribute refuses a test module's own
          # literals, which is loud and wrong rather than silent and wrong.
          loose="$(find src -name '*.rs' \
            ! -path 'src/view/palette.rs' \
            ! -path 'src/view/sgr.rs' |
            sort | xargs awk '
              FNR == 1 { in_test = 0; pending = 0 }
              pending && (/^[ \t]*$/ || /^[ \t]*\/\// || /^#\[/) { next }
              pending { pending = 0; if (/\{[ \t]*$/) { in_test = 1 }; next }
              /^#\[cfg\(test\)\]/ { pending = 1; next }
              in_test && /^\}/ { in_test = 0; next }
              in_test { next }
              /Color::|Modifier::/ {
                printf "  %s:%d: %s\n", FILENAME, FNR, $0
              }
            ')"

          if [ -n "$loose" ]; then
            echo "These lines name a colour or a weight outside the palette:"
            printf '%s\n' "$loose"
            echo
            echo "Give it a name in src/view/palette.rs saying what it means,"
            echo "and draw through that. A slot is a Style, so a weight is a"
            echo "slot too. Two slots may hold one value where they are two"
            echo "claims."
            exit 1
          fi
        '';

        # Every module in this tree already draws through the palette, so the
        # check above passes whether or not it can still find a literal. This
        # is what says it can — and that each of the three things it lets
        # through is let through on purpose rather than missed.
        coloursComeFromThePaletteTest = pkgs.runCommand "colours-come-from-the-palette-test"
          { nativeBuildInputs = [ coloursComeFromThePalette ]; } ''
          set -u

          tree="$TMPDIR/tree"
          mkdir -p "$tree/src/view"
          drawn="$tree/src/view/draw.rs"
          printf '//! A module that draws.\n\nfn plain() {}\n' > "$drawn"
          printf '//! The palette.\nconst QUIET: Style = Style::new().fg(Color::DarkGray);\n' \
            > "$tree/src/view/palette.rs"
          printf '//! A pane rewound.\nfn replay() { Color::Red; }\n' > "$tree/src/view/sgr.rs"

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }

          output="$( colours-come-from-the-palette "$tree" 2>&1 )" && status=0 || status=$?
          [ "$status" = 0 ] ||
            fail "it refused a tree whose only literals are the palette's and sgr.rs's:"

          # A production line, which is the whole point.
          printf '//! A module that draws.\n\nfn plain() { Color::DarkGray; }\n' > "$drawn"
          output="$( colours-come-from-the-palette "$tree" 2>&1 )" && status=0 || status=$?
          [ "$status" = 1 ] || fail "expected a refusal (exit 1), got $status:"
          # Read the listing rather than the whole output: the advice under
          # it names src/view/palette.rs, so a pattern over everything the
          # check said matches the exempt module in every run alike.
          named="$( printf '%s\n' "$output" | grep '^  src/' )"
          case "$named" in
            *src/view/draw.rs:3*) ;;
            *) fail "the refusal did not name the line:" ;;
          esac
          count="$( printf '%s\n' "$named" | grep -c . )"
          [ "$count" = 1 ] ||
            fail "one production line, so one line listed, got $count:"

          # A weight, which a palette of Colors could not have held.
          printf '//! A module that draws.\n\nfn plain() { Modifier::BOLD; }\n' > "$drawn"
          output="$( colours-come-from-the-palette "$tree" 2>&1 )" && status=0 || status=$?
          [ "$status" = 1 ] || fail "it allowed a Modifier outside the palette:"

          # The same literal inside a test module, which is where a value is
          # pinned rather than chosen.
          printf '//! A module that draws.\n\n#[cfg(test)]\nmod tests {\n    fn t() { Color::DarkGray; }\n}\n' \
            > "$drawn"
          output="$( colours-come-from-the-palette "$tree" 2>&1 )" && status=0 || status=$?
          [ "$status" = 0 ] || fail "it refused a literal inside a test module:"

          # A second attribute between `#[cfg(test)]` and the module it is
          # on. The item is still what says where the block opens.
          printf '//! A module that draws.\n\n#[cfg(test)]\n#[allow(clippy::pedantic)]\nmod tests {\n    fn t() { Color::DarkGray; }\n}\n' \
            > "$drawn"
          output="$( colours-come-from-the-palette "$tree" 2>&1 )" && status=0 || status=$?
          [ "$status" = 0 ] ||
            fail "an attribute under #[cfg(test)] hid the test module from it:"

          # The attribute above a bare declaration opens no block, so what
          # follows is production and is still read. Skipping to the next
          # closing brace here would swallow the rest of the file.
          printf '//! A module that draws.\n\n#[cfg(test)]\nmod fixtures;\n\nfn plain() { Color::DarkGray; }\n' \
            > "$drawn"
          output="$( colours-come-from-the-palette "$tree" 2>&1 )" && status=0 || status=$?
          [ "$status" = 1 ] ||
            fail "a bare #[cfg(test)] declaration hid the production line under it:"

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
            # Attributes stack on the item below them, so one written between
            # the marker and the module it marks does not end the run.
            if (line ~ /^ *#\[cfg\(test\)\]$/) {
              after_cfg_test = 1
            } else if (line !~ /^ *#\[/) {
              after_cfg_test = 0
            }

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

          # A test module can also live in a file of its own, declared
          # `#[cfg(test)] mod name;` where an inline one would have opened a
          # brace. That file is a test all through, and so is everything below
          # it: Rust puts a module's descendants in a directory named after
          # it, so the subtree is the module and a submodule added later
          # arrives already covered.
          declaring='
          function moduledir(path,   dir) {
            dir = path
            if (!sub(/\/(mod|lib|main)\.rs$/, "", dir)) {
              sub(/\.rs$/, "", dir)
            }
            return dir
          }
          function named(text,   name) {
            name = text
            sub(/^.*mod /, "", name)
            sub(/[;{ ]*$/, "", name)
            return name
          }
          # A declaration inside an inline module names a file below that
          # module rather than below the file it is written in, so where it
          # points is the declaration and everything open around it.
          function enclosing(ind,   i, dir) {
            dir = moduledir(FILENAME)
            for (i = 0; i < ind; i++) {
              if (opened[i] != "") {
                dir = dir "/" opened[i]
              }
            }
            return dir
          }
          {
            if ($0 ~ /^[ \t]*$/ || $0 ~ /^ *\/\//) {
              next
            }

            match($0, /^ */)
            ind = RLENGTH

            for (i = ind; i <= deepest; i++) {
              opened[i] = ""
            }

            if ($0 ~ /^ *(pub(\([a-z]+\))? )?mod [A-Za-z_0-9]+ \{$/) {
              opened[ind] = named($0)
              if (ind > deepest) {
                deepest = ind
              }
            } else if (after_cfg_test && $0 ~ /^ *(pub(\([a-z]+\))? )?mod [A-Za-z_0-9]+;$/) {
              print enclosing(ind) "/" named($0)
            }

            if ($0 ~ /^ *#\[cfg\(test\)\]$/) {
              after_cfg_test = 1
            } else if ($0 !~ /^ *#\[/) {
              after_cfg_test = 0
            }
          }
          '

          own_file="$(
            find src -name '*.rs' | sort | while IFS= read -r module; do
              ${pkgs.gawk}/bin/gawk "$declaring" "$module"
            done | while IFS= read -r root; do
              if [ -f "$root.rs" ]; then
                echo "$root.rs"
              fi
              if [ -d "$root" ]; then
                find "$root" -name '*.rs'
              fi
            done | sort -u
          )"

          # Under src/ the tests are the `#[cfg(test)]` modules inside the file
          # they cover, and the files the ones above live in; everything under
          # tests/ is a test all through.
          loose="$( {
            find src -name '*.rs' | sort | while IFS= read -r module; do
              if printf '%s\n' "$own_file" | grep -qxF -- "$module"; then
                whole=1
              else
                whole=0
              fi
              ${pkgs.gawk}/bin/gawk -v whole_file="$whole" "$reading" "$module"
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

          # A test module in a file of its own, which is what the check could
          # not see into. `inline.rs` carries the same walk where the check has
          # always reached, so a quiet `painted.rs` cannot be read as a check
          # that stopped working; `live.rs` is the sibling in the same
          # directory declared without `#[cfg(test)]`, which says the file is
          # read as a test because of its declaration and not its neighbours.
          mkdir -p "$tree/src/view"
          {
            echo "#[cfg(test)]"
            echo "pub(crate) mod painted;"
            echo ""
            echo "#[cfg(test)]"
            echo "#[allow(dead_code)]"
            echo "pub(crate) mod stacked;"
            echo ""
            echo "pub(crate) mod inline;"
            echo "pub(crate) mod live;"
          } > "$tree/src/view/mod.rs"
          {
            echo "//! A test module that lives in a file of its own."
            echo "fn walk(shown: &mut Shown) {"
            echo "    while shown.apply(Action::Move(Motion::NextRow)) {}"
            echo "}"
          } > "$tree/src/view/painted.rs"
          {
            echo "//! The same walk, where the check has always reached."
            echo "#[cfg(test)]"
            echo "mod tests {"
            echo "    fn walk(shown: &mut Shown) {"
            echo "        while shown.apply(Action::Move(Motion::NextRow)) {}"
            echo "    }"
            echo "}"
          } > "$tree/src/view/inline.rs"
          {
            echo "//! A sibling declared without #[cfg(test)]: production."
            echo "fn settle(&mut self) {"
            echo "    while self.apply(Action::Refresh) {"
            echo "        self.lay_out();"
            echo "    }"
            echo "}"
          } > "$tree/src/view/live.rs"

          # Declared inside an inline module, so Rust looks for it below that
          # module and not below the file the declaration is written in. A
          # pass that read the declaration alone would name a file that is
          # not there and leave the one that is unscanned.
          mkdir -p "$tree/src/view/nested/outer"
          {
            echo "//! A file-backed test module declared inside an inline one."
            echo "pub(crate) mod outer {"
            echo "    #[cfg(test)]"
            echo "    mod deep;"
            echo "}"
          } > "$tree/src/view/nested.rs"
          {
            echo "//! Below the inline module that declares it."
            echo "fn walk(shown: &mut Shown) {"
            echo "    while shown.apply(Action::Move(Motion::NextRow)) {}"
            echo "}"
          } > "$tree/src/view/nested/outer/deep.rs"

          # An attribute of its own between the marker and the module it
          # marks, which is legal on either kind and reads to a scanner as
          # something else standing where the module should have been.
          {
            echo "//! An attribute between the marker and an inline module."
            echo "#[cfg(test)]"
            echo "#[allow(dead_code)]"
            echo "mod tests {"
            echo "    fn walk(shown: &mut Shown) {"
            echo "        while shown.apply(Action::Move(Motion::NextRow)) {}"
            echo "    }"
            echo "}"
          } > "$tree/src/attributed.rs"
          {
            echo "//! An attribute between the marker and a file of its own."
            echo "fn walk(shown: &mut Shown) {"
            echo "    while shown.apply(Action::Move(Motion::NextRow)) {}"
            echo "}"
          } > "$tree/src/view/stacked.rs"

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
            *"src/view/painted.rs:3"*) ;;
            *) fail "the refusal did not reach a test module in a file of its own:" ;;
          esac
          case "$output" in
            *"src/view/inline.rs:5"*) ;;
            *) fail "the refusal did not reach the inline walk beside it:" ;;
          esac
          case "$output" in
            *"src/view/nested/outer/deep.rs:3"*) ;;
            *) fail "the refusal did not follow a declaration into an inline module:" ;;
          esac
          case "$output" in
            *"src/attributed.rs:6"*) ;;
            *) fail "an attribute after the marker hid an inline module:" ;;
          esac
          case "$output" in
            *"src/view/stacked.rs:3"*) ;;
            *) fail "an attribute after the marker hid a module in its own file:" ;;
          esac
          case "$output" in
            *counted.rs*) fail "it named a walk that is counted out before it starts:" ;;
          esac
          case "$output" in
            *live.rs*) fail "it named a sibling declared without #[cfg(test)]:" ;;
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

          # `view/mod.rs` and `live.rs` stay, so the green below is also what
          # says the check reads a declaration that no longer resolves to a
          # file without complaint.
          rm "$tree/src/loose.rs" "$tree/src/split.rs"
          rm "$tree/src/deadline.rs" "$tree/tests/integration.rs"
          rm "$tree/src/view/painted.rs" "$tree/src/view/inline.rs"
          rm "$tree/src/view/nested/outer/deep.rs"
          rm "$tree/src/attributed.rs" "$tree/src/view/stacked.rs"
          screen-walks-are-bounded "$tree" ||
            fail "it refused a tree in which every walk is counted out:"

          touch $out
        '';

        # The version is written in three places and two of them are held to
        # the crate: `flake.nix` reads it out of `Cargo.toml`, and `cargo
        # publish --locked` refuses a `Cargo.lock` that disagrees. README's
        # flake example is the third, and a bump that forgets it lands green —
        # then the release ships a page telling a reader to pin the tag before
        # the one being released.
        #
        # A pin is any ref written after the repository, because
        # `github:owner/repo` alone is a reader taking whatever is on main,
        # which is what the lines above the example tell them to do. What comes
        # after it has to be the tag for the version the crate declares: a
        # branch or a sha resolves and builds, so nothing else here would ever
        # notice one.
        readmePinsTheVersion = pkgs.writeShellScriptBin "readme-pins-the-version" ''
          set -u

          cd "''${1:-.}" || exit 1

          version="$(awk -F'"' '
            /^\[/ { package = ($0 == "[package]") }
            package && /^version *=/ { print $2; exit }
          ' Cargo.toml)"

          pins="$(grep -oE 'github:CodeForBreakfast/beady-eye/[^"[:space:]]+' README.md |
            sed 's|^github:CodeForBreakfast/beady-eye/||' | sort -u)"

          if [ -z "$pins" ]; then
            echo "README no longer pins the flake input to a release tag."
            echo
            echo "The example a reader copies is the only place the page says to"
            echo "pin one, and the versioning section under Status points back at"
            echo "it. Put a github:CodeForBreakfast/beady-eye/v$version back."
            exit 1
          fi

          # Whoever meets this meets it in a CI log with the guard unread, so
          # each version is on a line naming the file it came from. A sentence
          # holding both wraps, and then a value stands under a label that
          # belongs to the other file.
          wrong="$(printf '%s\n' "$pins" | grep -vxF "v$version" | sed 's/^/  /')"
          if [ -n "$wrong" ]; then
            echo "README pins the flake input to something the crate does not declare."
            echo
            echo "Cargo.toml declares $version."
            echo "README pins:"
            printf '%s\n' "$wrong"
            echo
            echo "So the example reads github:CodeForBreakfast/beady-eye/v$version."
            exit 1
          fi
        '';

        # The README in this tree pins the version it declares, so the check
        # above passes whether or not it can still refuse one. This is what
        # says it can — and that the unpinned references the page opens with
        # are let through on purpose rather than missed.
        readmePinsTheVersionTest = pkgs.runCommand "readme-pins-the-version-test"
          { nativeBuildInputs = [ readmePinsTheVersion ]; } ''
          set -u

          tree="$TMPDIR/tree"
          mkdir -p "$tree"
          printf '[package]\nname = "beady-eye"\nversion = "0.2.0"\n' > "$tree/Cargo.toml"

          readme() { printf '%s\n' "$1" > "$tree/README.md"; }
          pinned='inputs.beady-eye.url = "github:CodeForBreakfast/beady-eye/v0.2.0";'
          unpinned='$ nix run github:CodeForBreakfast/beady-eye'

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }

          accepts() {
            output="$( readme-pins-the-version "$tree" 2>&1 )" && status=0 || status=$?
            [ "$status" = 0 ] || fail "$1"
          }

          refuses() {
            output="$( readme-pins-the-version "$tree" 2>&1 )" && status=0 || status=$?
            [ "$status" = 1 ] || fail "expected a refusal (exit 1), got $status: $1"
            case "$output" in
              *"$2"*) ;;
              *) fail "the refusal did not say why ($2): $1" ;;
            esac
          }

          readme "$pinned"
          accepts "it refused a README pinning the version the crate declares:"

          # The bump that forgets the README, which is the whole of this.
          readme 'inputs.beady-eye.url = "github:CodeForBreakfast/beady-eye/v0.1.0";'
          refuses "it accepted a pin naming another version:" "v0.1.0"
          readme 'inputs.beady-eye.url = "github:CodeForBreakfast/beady-eye/v0.1.0";'
          refuses "the refusal did not name the version the crate declares:" "0.2.0"

          # A pin is a release tag rather than any ref that resolves. A branch
          # builds, so nothing else here would notice.
          readme 'inputs.beady-eye.url = "github:CodeForBreakfast/beady-eye/main";'
          refuses "it accepted a pin that is not a release tag:" "main"

          # The README with the example taken out of it. Every reading above
          # passes on a page that pins nothing, so this is what holds the
          # example in place rather than merely holding it right.
          readme "$unpinned"
          refuses "it accepted a README that pins nothing at all:" "no longer"

          # The page opens by telling a reader to run the flake without pinning
          # it, five times. Read as pins those would be five refusals on the
          # tree as it stands.
          readme "$unpinned
$pinned"
          accepts "an unpinned reference was read as a pin:"

          touch $out
        '';

        # The Release body is RELEASE-NOTES/<version>.md byte for byte, and
        # GitHub renders that body with its hard line break extension on — so a
        # newline inside a paragraph becomes a <br> and the published page
        # shows the file's wrap as ragged short lines. Every other document in
        # this tree is wrapped, so a notes file written like its neighbours is
        # the mistake.
        #
        # A wrap is a line whose successor continues it. Inside a fence the
        # breaks are the content, and RELEASE-NOTES/README.md is a repository
        # document rather than a Release body, so the glob takes the version
        # files only.
        releaseNotesAreUnwrapped = pkgs.writeShellScriptBin "release-notes-are-unwrapped" ''
          set -u

          cd "''${1:-.}" || exit 1

          files=""
          for file in RELEASE-NOTES/[0-9]*.md; do
            [ -e "$file" ] && files="$files $file"
          done

          if [ -z "$files" ]; then
            echo "RELEASE-NOTES/ carries no version file."
            echo
            echo "Every release cuts its body from RELEASE-NOTES/<version>.md, so a"
            echo "scan that reads nothing means the fileset or the name has moved,"
            echo "rather than that the notes are clean."
            exit 1
          fi

          wrapped="$(awk '
            FNR == 1 { fence = 0; held = "" }

            /^```/ { fence = !fence; held = ""; next }
            fence  { next }

            /^[[:space:]]*$/ { held = ""; next }

            {
              starts = /^#/ || /^[[:space:]]*[-*+] / ||
                       /^[[:space:]]*[0-9]+[.)] / || /^>/ || /^\|/ ||
                       /^(-{3,}|\*{3,}|_{3,})$/
              if (held != "" && !starts) print "  " FILENAME ":" heldno ": " held
              held = $0
              heldno = FNR
            }
          ' $files)"

          if [ -n "$wrapped" ]; then
            echo "A release notes file is wrapped, and its Release page shows the wrap."
            echo
            echo "GitHub renders a Release body with hard line breaks on, so every"
            echo "newline inside a paragraph becomes a <br>. Put each paragraph and"
            echo "each bullet on one line; a fenced block keeps its own breaks."
            echo
            echo "Lines a wrap continues:"
            printf '%s\n' "$wrapped"
            exit 1
          fi
        '';

        # The notes files in this tree are unwrapped, so the check above passes
        # whether or not it can still refuse one. This is what says it can —
        # and that a list, a fence and the wrapped README beside them are let
        # through on purpose rather than missed.
        releaseNotesAreUnwrappedTest = pkgs.runCommand "release-notes-are-unwrapped-test"
          { nativeBuildInputs = [ releaseNotesAreUnwrapped ]; } ''
          set -u

          tree="$TMPDIR/tree"
          mkdir -p "$tree/RELEASE-NOTES"

          notes() { cat > "$tree/RELEASE-NOTES/1.0.0.md"; }
          readme() { cat > "$tree/RELEASE-NOTES/README.md"; }

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }

          accepts() {
            output="$( release-notes-are-unwrapped "$tree" 2>&1 )" && status=0 || status=$?
            [ "$status" = 0 ] || fail "$1"
          }

          refuses() {
            output="$( release-notes-are-unwrapped "$tree" 2>&1 )" && status=0 || status=$?
            [ "$status" = 1 ] || fail "expected a refusal (exit 1), got $status: $1"
            case "$output" in
              *"$2"*) ;;
              *) fail "the refusal did not say why ($2): $1" ;;
            esac
          }

          # Nothing to read is not the same as clean files.
          refuses "it accepted a RELEASE-NOTES holding no version file:" "no version file"

          notes <<'EOF'
bdi 1.0.0

Major release, **0.9.0 → 1.0.0**.

## Highlights

**The tracker is read once per keypress.** A collection of nine trackers no longer costs nine reads to move the selection.

## Maintenance

- The lockfile, and the two crates behind it.
- A shim nothing called any more.
EOF
          accepts "it refused a file that is already one line per paragraph:"

          # The wrap this whole check exists for.
          notes <<'EOF'
bdi 1.0.0

**The tracker is read once per keypress.** A collection of nine trackers
no longer costs nine reads to move the selection.
EOF
          refuses "it accepted a wrapped paragraph:" "wrapped"
          refuses "the refusal did not name the line the wrap continues:" "once per keypress"

          # A continuation indented under a bullet is a wrap as much as a
          # paragraph is, and it is the one a writer produces without noticing.
          notes <<'EOF'
## Maintenance

- The lockfile, and the two crates behind it, neither of which changes
  anything a reader would see.
EOF
          refuses "it accepted a wrapped bullet:" "The lockfile"

          # Consecutive bullets are the false positive that would make this
          # check unusable, because every list is a run of non-blank lines.
          notes <<'EOF'
## Maintenance

- The lockfile, and the two crates behind it.
- A shim nothing called any more.
- The pty tests, which now drain across the reap.
EOF
          accepts "a list was read as a wrapped paragraph:"

          # A nested item is indented like the continuation of a wrapped
          # bullet, so a marker is what tells the two apart rather than the
          # indent.
          notes <<'EOF'
## Maintenance

- The lockfile, and the two crates behind it.
  - One of them reached a major version.
EOF
          accepts "a nested list was read as a wrapped bullet:"

          notes <<'EOF'
## Taking a binary

```console
$ curl -fLO https://example.invalid/bdi
$ chmod +x bdi
$ ./bdi --version
```
EOF
          accepts "a fenced block was read as a wrapped paragraph:"

          # Reading the wrapped README beside the notes would refuse the tree
          # as it stands.
          readme <<'EOF'
One file per release, `RELEASE-NOTES/<version>.md`, where `<version>` is
the `MAJOR.MINOR.PATCH` the release bumps to.
EOF
          accepts "it read the wrapped README beside the notes files:"

          touch $out
        '';


        # The subject main carries comes from the pull request's title, because
        # this repository squash-merges — so the title is the one string worth
        # refusing, and a branch's own commit messages are squashed away.
        #
        # It cannot be a `checks` entry the way the scans above are. Those read
        # a tree, and neither a commit message nor a pull request title is in
        # one; this is handed a string by the workflow instead. What stays here
        # is the rule and its test, so the only thing ci.yml holds is the
        # trigger.
        conventionalSubject = pkgs.writeShellScriptBin "conventional-subject" ''
          set -u

          title="''${1-}"

          if [ "$#" -ne 1 ]; then
            echo "usage: conventional-subject <title>" >&2
            exit 2
          fi

          types='build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test'
          scopes='collect|app|model|view|tui|ci|flake|docs|tests|deps'

          refuse() {
            echo "This is not a subject main can carry:"
            echo
            echo "  $title"
            echo
            printf '%s\n' "$1"
            exit 1
          }

          # A newline in the title is a body that has been pasted into the
          # subject. Only the line endings are refused: a high byte is not a
          # control character here, and an em dash is ordinary in this repo.
          newline='
'
          carriageReturn="$( printf '\r' )"
          case "$title" in
            *"$newline"* | *"$carriageReturn"*)
              refuse "It has a newline in it, so it is a subject and a body."
              ;;
          esac

          # Read in either case, so a wrong case is told apart from a wrong
          # shape: rule 15 of the specification binds a parser to recognise
          # `Fix:` as the type `fix`, and config-conventional's
          # `type-case: lower-case` is what refuses it afterwards.
          #
          # The character after the colon and space has to be one. A
          # description opening with a blank leaves `firstWord` empty, and an
          # empty first word matches neither the case pattern nor the list of
          # moods, so both of the checks below would pass by describing
          # nothing.
          if ! printf '%s' "$title" | grep -Eq '^[A-Za-z]+(\([A-Za-z0-9_-]+\))?!?: [^[:space:]]'; then
            refuse "Write it as \`type(scope): description\`, or \`type: description\`.
A \`!\` before the colon marks a breaking change.

  types:  ''${types//|/ }
  scopes: ''${scopes//|/ }"
          fi

          type="$( printf '%s' "$title" | sed -E 's/^([A-Za-z]+).*/\1/' )"
          scope="$( printf '%s' "$title" | sed -nE 's/^[A-Za-z]+\(([^)]*)\).*/\1/p' )"
          description="''${title#*: }"

          if printf '%s' "$type$scope" | grep -q '[A-Z]'; then
            refuse "A type and a scope are lower case: \`''${type,,}''${scope:+(''${scope,,})}\`."
          fi

          if ! printf '%s' "$type" | grep -Eq "^($types)$"; then
            refuse "\`$type\` is not one of the types: ''${types//|/ }"
          fi

          if [ -n "$scope" ] && ! printf '%s' "$scope" | grep -Eq "^($scopes)$"; then
            refuse "\`$scope\` is not one of the scopes: ''${scopes//|/ }

The first five are the layers in src/. Leave the scope out rather than
coin one; adding a scope means adding it here too."
          fi

          # subject-full-stop: never. The subject is a label rather than a
          # sentence, and the body is where the sentences go.
          case "$description" in
            *.) refuse "A description does not end in a full stop." ;;
          esac

          # subject-case: never sentence-case. Only an ordinary capitalised
          # first word is refused, so an acronym or a name keeps its capitals
          # — GitHub, CI, README and NO_COLOR all read as written.
          firstWord="''${description%% *}"
          if printf '%s' "$firstWord" | grep -Eq '^[A-Z][a-z]*$'; then
            refuse "A description starts lower case: \`''${firstWord,,}\`, not \`$firstWord\`.
An acronym or a name keeps its capitals."
          fi

          # Imperative mood, which no pattern can read. A suffix cannot stand
          # in for it — `-ing` would refuse `bring`, and `-ed` would refuse
          # `read`, `seed` and `feed`. So the forms that turn up are named
          # instead, which makes this a floor rather than a judge of mood.
          notImperative='adds|added|adding
            addresses|addressed|addressing
            adjusts|adjusted|adjusting
            allows|allowed|allowing
            bumps|bumped|bumping
            changes|changed|changing
            cleans|cleaned|cleaning
            converts|converted|converting
            corrects|corrected|correcting
            creates|created|creating
            deletes|deleted|deleting
            drops|dropped|dropping
            ensures|ensured|ensuring
            extracts|extracted|extracting
            fixes|fixed|fixing
            handles|handled|handling
            implements|implemented|implementing
            improves|improved|improving
            includes|included|including
            introduces|introduced|introducing
            makes|made|making
            merges|merged|merging
            moves|moved|moving
            prevents|prevented|preventing
            refactors|refactored|refactoring
            removes|removed|removing
            renames|renamed|renaming
            replaces|replaced|replacing
            reverts|reverted|reverting
            simplifies|simplified|simplifying
            supports|supported|supporting
            updates|updated|updating
            uses|used|using
            writes|wrote|writing'
          notImperative="$( printf '%s' "$notImperative" | tr -d ' ' | tr '\n' '|' )"
          if printf '%s' "$firstWord" | grep -Eq "^($notImperative)$"; then
            refuse "\`$firstWord\` is not the imperative. A description has to finish the
sentence \"If applied, this commit will …\", so \`draw\` rather than
\`draws\`, \`drew\` or \`drawing\`."
          fi

          limit=72
          if [ "''${#title}" -gt "$limit" ]; then
            refuse "It is ''${#title} characters. The limit is $limit.

Say what changed here and put the reason in the body, in a sentence or
two. A subject this long prints over two lines in \`git log --oneline\`,
in blame and in bisect."
          fi
        '';

        # Every subject this repository takes goes through the check above, so
        # it passes whether or not it can still refuse one. This is what says
        # it can, and that each thing it lets through is let through on
        # purpose.
        conventionalSubjectTest = pkgs.runCommand "conventional-subject-test"
          { nativeBuildInputs = [ conventionalSubject ]; } ''
          set -u

          fail() { echo "FAIL: $1"; echo "$output"; exit 1; }

          accepts() {
            output="$( conventional-subject "$@" 2>&1 )" && status=0 || status=$?
            [ "$status" = 0 ] || fail "it refused \`$1\`:"
          }

          refuses() {
            want="$1"
            shift
            output="$( conventional-subject "$@" 2>&1 )" && status=0 || status=$?
            [ "$status" = 1 ] || fail "expected a refusal (exit 1) of \`$1\`, got $status:"
            case "$output" in
              *"$want"*) ;;
              *) fail "the refusal of \`$1\` did not say why ($want):" ;;
            esac
          }

          # A scope, and no scope, which conventional commits leaves optional.
          accepts "feat(view): draw a bead id in its status colour"
          accepts "docs: say how a reader on a light background says so"

          # The breaking-change marker, with a scope and without.
          accepts "feat!: read every herdr session on the box"
          accepts "fix(collect)!: drop the ambient bd path"

          # Every type and every scope, so the two lists are asserted rather
          # than described. A list that grows without its test growing is the
          # drift this catches.
          for type in build chore ci docs feat fix perf refactor revert style test; do
            accepts "$type: a subject of no particular interest"
          done
          for scope in collect app model view tui ci flake docs tests deps; do
            accepts "fix($scope): a subject of no particular interest"
          done

          # The boundary, from both sides. 72 is the limit rather than the
          # first length refused.
          at72="fix(view): 00000000000000000000000000000000000000000000000000000000000ab"
          [ "''${#at72}" = 72 ] || fail "the fixture is ''${#at72} characters, not 72:"
          accepts "$at72"
          refuses "73 characters" "''${at72}c"

          # The title is the whole subject on this repository, so nothing is
          # added to it before the length is read. A second argument is not
          # accepted at all, rather than quietly ignored, so that a caller
          # still passing a pull request number is told instead of measuring
          # something this does not measure.
          output="$( conventional-subject "fix(view): draw the row" 133 2>&1 )" &&
            status=0 || status=$?
          [ "$status" = 2 ] || fail "expected exit 2 for a second argument, got $status:"

          # The shape.
          refuses "type(scope): description" "A read is a read of bdi's asking"
          refuses "type(scope): description" "fix(view):no space after the colon"
          refuses "type(scope): description" "fix(view): "

          # A description opening with a blank. The first word is then the
          # empty string, which matches neither the case pattern nor any mood
          # in the list — so both of the checks below would pass on a title
          # they are meant to refuse, and say nothing about it.
          refuses "type(scope): description" "fix:  Fixed the row it drew"
          refuses "type(scope): description" "fix(view):  fixes the row"

          # Case. The specification says a parser reads `Fix:` as the type
          # `fix`, so this reads it as one and then refuses it on
          # config-conventional's `type-case: lower-case` — which is a
          # different complaint from a subject that is not a conventional
          # commit at all, and has to say so.
          refuses "lower case" "Fix(view): draw the row"
          refuses "lower case" "fix(View): draw the row"

          # subject-full-stop, and subject-case. A capitalised ordinary word
          # opens none of these; an acronym or a name opens three of them and
          # must survive, or half this repository's vocabulary is unwritable.
          refuses "full stop" "fix(view): draw the row."
          refuses "starts lower case" "fix(view): The reader says what it is"
          refuses "starts lower case" "fix(view): A read is a read of bdi's asking"
          accepts "fix(view): GitHub appends the reference as it squashes"
          accepts "fix(ci): CI seeds its own checks"
          accepts "fix(view): NO_COLOR survives the tail's voice"
          accepts "docs(flake): README says what bdi needs"

          # The imperative. What it refuses are the forms named in the list,
          # and what it must not refuse is an imperative that merely looks like
          # one — `read`, `seed` and `feed` all end in the suffix a lazier
          # check would have used, and `bring` in the other one.
          refuses "not the imperative" "fix(view): fixes the row it drew"
          refuses "not the imperative" "fix(view): fixed the row it drew"
          refuses "not the imperative" "fix(view): fixing the row it drew"
          refuses "not the imperative" "fix(view): updates the badge"
          refuses "not the imperative" "fix(view): made the badge quieter"
          accepts "fix(view): read the row back before drawing it"
          accepts "ci(flake): seed the dependency builds into the cache"
          accepts "fix(collect): feed bd the project it asked about"
          accepts "fix(view): bring the selection back to the fold"

          # A type and a scope outside their lists. Both are the drift the
          # closed sets exist to stop, and each has to name itself.
          refuses "\`improve\` is not one of the types" "improve(view): sharpen a row"
          refuses "\`bead-window\` is not one of the scopes" "fix(bead-window): keep the page"

          # A newline would put a body in the subject. An em dash must not be
          # refused with it: this repository's prose is full of them.
          refuses "newline" "fix(view): a subject
and a second line"
          accepts "fix(view): a subject — with an em dash in it"

          # No argument at all is a misuse of the check rather than a bad
          # subject, and the workflow should not read it as one.
          output="$( conventional-subject 2>&1 )" && status=0 || status=$?
          [ "$status" = 2 ] || fail "expected exit 2 with no argument, got $status:"

          touch $out
        '';

        # The two questions a run's early exit turns on, kept out of the
        # command that asks them so a test can put every answer git and gh
        # can give in front of the decision itself.
        alreadyJudged = ''
          jq=${pkgs.jq}/bin/jq
          sed=${pkgs.gnused}/bin/sed

          # The pull request a squash names, out of the subject GitHub built
          # from its title. Empty where the subject names none, which is a
          # commit that reached main by some route that left no run to read.
          pull_request_in() {
            printf '%s\n' "$1" | $sed -n 's/.*(#\([0-9][0-9]*\))$/\1/p'
          }

          # What `gh run list --workflow ci.yml --commit <head>` said, on
          # stdin. Only that head's own pull request runs answer: a commit can
          # be a branch tip as well, and a run reached that way was handed the
          # tree at the tip rather than the merge of this head with its base.
          #
          # One head carries several runs whenever a title was edited after CI
          # started, and the concurrency group cancels the one it superseded.
          # So this asks whether any run passed rather than what the newest
          # one concluded, and only "success" is a pass — a conclusion GitHub
          # has not invented yet comes back "refused" rather than falling
          # through to one.
          head_run_state() {
            $jq -r --arg sha "$1" '
              [.[] | select(.event == "pull_request" and .headSha == $sha)] as $runs |
              if   ($runs | length) == 0 then "none"
              elif any($runs[]; .status == "completed" and .conclusion == "success") then "success"
              elif any($runs[]; .status != "completed") then "running"
              else "refused" end'
          }
        '';

        # ci.yml calls this where the step used to hold the shell itself, so
        # what decides that a tree is not built is a check in the flake rather
        # than a script only a run ever executes. The workflow keeps the half
        # a tree cannot hold: the event, and the token gh reads.
        #
        # A tree that asks for the very derivations something has already been
        # given a verdict on can only arrive at that same verdict, so the run
        # does not build them. Which trees those are is a question for the
        # flake rather than a list of documentation paths kept in a workflow: a
        # derivation path answers for the source, for flake.nix and for
        # flake.lock at once, so a dependency bump or an edited check is caught
        # by construction.
        #
        # The verdict travels with the derivation rather than with the run that
        # reported it. Two equal drvPath sets are the same build and not a
        # similar one, so a skip here does not weaken what a green
        # `nix flake check` job means: every check the flake declares for that
        # tree has passed. What it stops meaning is that this particular run is
        # where the building happened.
        unbuiltChecks = pkgs.writeShellScriptBin "unbuilt-checks" ''
          set -u

          git=${pkgs.git}/bin/git
          gh=${pkgs.gh}/bin/gh
          mktemp=${pkgs.coreutils}/bin/mktemp
          tar=${pkgs.gnutar}/bin/tar

          ${alreadyJudged}

          case "''${1:-}" in
            -h|--help)
              cat <<'USAGE'
          unbuilt-checks

          Prints "false" where every check this tree declares is a derivation
          something has already had a verdict on, and "true" otherwise. Which
          tree that verdict came from depends on GITHUB_EVENT_NAME:

            pull_request   the branch it targets, whose own push run built it.
            push           the head of the pull request the subject names,
                           whose run main's ruleset required green before the
                           squash could land.

          Anything it cannot decide answers "true", so an unreadable subject,
          an unreachable head and a gh that will not answer each cost a needless
          build rather than leaving a tree nothing has checked.
          USAGE
              exit 0
              ;;
          esac

          if [ "$#" -ne 0 ]; then
            echo "unbuilt-checks: takes no argument, and was given: $*" >&2
            exit 2
          fi

          # Every way out prints one word. The reason goes to stderr, where the
          # run log keeps it beside the decision it explains.
          builds() {
            echo "$1" >&2
            echo true
            exit 0
          }

          event="''${GITHUB_EVENT_NAME:-}"
          case "$event" in
            pull_request|push) ;;
            *) builds "Nothing gives a '$event' event's tree a verdict in advance." ;;
          esac

          cd "$($git rev-parse --show-toplevel)" || exit 1

          if [ "$event" = pull_request ]; then
            judged=origin/main
          else
            number="$(pull_request_in "$($git log -1 --format=%s)")"
            [ -n "$number" ] ||
              builds "This commit's subject names no pull request, so nothing has judged its tree."

            # The merge ref a pull request's run was given is deleted when the
            # pull request merges, so the head is what is left to compare
            # against.
            $git fetch --quiet --no-tags origin "refs/pull/$number/head" ||
              builds "Pull request #$number's head could not be fetched, so its verdict cannot be read."
            judged=FETCH_HEAD

            head="$($git rev-parse FETCH_HEAD)"

            # A head's green is not on its own a verdict on that head's tree:
            # the run was given the merge of it with main as main stood at the
            # time. Where the head contains this commit's parent it is the
            # same tree either way — main only moves forward, so a head
            # holding the parent holds everything main had while the run was
            # going, and the merge the run was handed was the head's own tree.
            #
            # Where the head does not contain it, the branch never saw what
            # main did and that run was given a tree this squash is not. A
            # change on main and its revert straddling a pull request is the
            # shape that reaches here with matching derivations and no build
            # behind them, and this is what refuses it.
            parent="$($git rev-parse --verify --quiet HEAD^)" ||
              builds "This commit has no parent, so nothing here says what main held while that run was going."
            $git merge-base --is-ancestor "$parent" "$head" ||
              builds "Pull request #$number's head does not contain $parent, so its run was given a tree this one is not."
            runs="$($gh run list --workflow ci.yml --commit "$head" --limit 50 \
                      --json databaseId,headSha,status,conclusion,event)" ||
              builds "gh would not say what CI made of pull request #$number's head."

            state="$(printf '%s' "$runs" | head_run_state "$head")"
            [ "$state" = success ] ||
              builds "CI's verdict on pull request #$number's head $head reads $state rather than success."
          fi

          # The nix that evaluates this is the one that ran this script, since
          # it is the same nix that has to build what the answer does not skip.
          derivations() {
            nix eval --json "$1#checks.${system}" \
              --apply 'builtins.mapAttrs (_: check: check.drvPath)'
          }

          # Unpacked rather than checked out, so what is evaluated is the
          # committed tree and nothing this run has since put beside it.
          base="$($mktemp -d)"
          $git archive "$judged" | $tar -x -C "$base" ||
            builds "$judged's tree could not be unpacked."

          here="$(derivations .)" ||
            builds "This tree's checks could not be evaluated."
          [ -n "$here" ] ||
            builds "This tree declares no check, which is not a tree to skip."
          there="$(derivations "path:$base")" ||
            builds "$judged's checks could not be evaluated."

          if [ "$here" = "$there" ]; then
            echo "Every check here is a derivation $judged has already had a verdict on." >&2
            echo false
          else
            echo true
          fi
        '';

        # ci.yml holds the trigger and this holds the decision, the way the
        # conventional-subject rule is split. A step is prose until a run
        # executes it, and the step this replaces decided whether a tree got
        # built with nothing checking that it decided right.
        unbuiltChecksTest = pkgs.runCommand "unbuilt-checks-test"
          { nativeBuildInputs = [ unbuiltChecks ]; } ''
          set -u
          ${alreadyJudged}

          sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
          other=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb

          fail() { echo "FAIL: $1"; exit 1; }

          names() {
            got="$(pull_request_in "$2")" || got="(sed refused it)"
            [ "$got" = "$1" ] ||
              fail "the pull request in '$2' read as '$got' rather than '$1'"
          }

          # A squash subject here is the pull request's title with the number
          # GitHub appends to it, so that is what the number is read out of.
          names 41 "feat(tui): retire a notice settled at startup after a minute (#41)"
          names 7 "fix: a subject (#7)"
          names 1234 "chore: a subject (#1234)"

          # A subject that mentions a pull request without being one, and the
          # shapes a commit that reached main some other way arrives in. Each
          # has to come back empty rather than send this to somebody else's
          # run for a verdict on a tree that is not this one.
          names "" "revert: undo what (#41) landed yesterday"
          names "" "fix: a subject"
          names "" "fix: a subject #41"
          names "" "fix: a subject (41)"
          names "" "fix: a subject (#) "

          pull_request_run() {
            printf '{"databaseId":1,"headSha":"%s","status":"%s","conclusion":%s,"event":"pull_request"}' \
              "$1" "$2" "$3"
          }

          push_run() {
            printf '{"databaseId":2,"headSha":"%s","status":"%s","conclusion":%s,"event":"push"}' \
              "$1" "$2" "$3"
          }

          reads() {
            want="$1"
            shift
            got="$(printf '[%s]' "$*" | head_run_state "$sha")" || got="(jq refused it)"
            [ "$got" = "$want" ] ||
              fail "a run list this expected to read as '$want' read as '$got': [$*]"
          }

          reads none
          reads success "$(pull_request_run "$sha" completed '"success"')"
          reads running "$(pull_request_run "$sha" in_progress null)"
          reads running "$(pull_request_run "$sha" queued null)"

          # Every other conclusion GitHub records, and the one it records for
          # a run that finished without reaching one. None of these is a tree
          # anything passed on.
          reads refused "$(pull_request_run "$sha" completed '"failure"')"
          reads refused "$(pull_request_run "$sha" completed '"cancelled"')"
          reads refused "$(pull_request_run "$sha" completed '"timed_out"')"
          reads refused "$(pull_request_run "$sha" completed '"skipped"')"
          reads refused "$(pull_request_run "$sha" completed null)"

          # A conclusion nobody here has heard of asks for a build rather than
          # being read as one, so the day GitHub adds one no tree skips on it.
          reads refused "$(pull_request_run "$sha" completed '"embargoed"')"

          # One head carries several runs whenever a title was edited after
          # CI started, and the concurrency group cancels the one it
          # superseded. Order is gh's, so neither position may be the one
          # that decides.
          reads success \
            "$(pull_request_run "$sha" completed '"cancelled"'), $(pull_request_run "$sha" completed '"success"')"
          reads success \
            "$(pull_request_run "$sha" completed '"success"'), $(pull_request_run "$sha" completed '"cancelled"')"

          # A run still going beside a green one does not unjudge the tree the
          # green one passed.
          reads success \
            "$(pull_request_run "$sha" completed '"success"'), $(pull_request_run "$sha" in_progress null)"

          # Somebody else's head. gh was asked about one commit and answering
          # with another is not a verdict on this tree.
          reads none "$(pull_request_run "$other" completed '"success"')"

          # The same commit reached by a push. That run was given the tree at
          # a branch tip rather than the merge of this head with its base, so
          # it says nothing about what a pull request was judged on.
          reads none "$(push_run "$sha" completed '"success"')"
          reads success \
            "$(push_run "$sha" completed '"failure"'), $(pull_request_run "$sha" completed '"success"')"

          # The three the command decides before it has looked at a tree.
          # Nothing here is in a git repository and none of these needs one.
          output="$( unbuilt-checks --help 2>&1 )" && status=0 || status=$?
          [ "$status" = 0 ] || fail "expected exit 0 for --help, got $status: $output"

          output="$( unbuilt-checks a-tree 2>&1 )" && status=0 || status=$?
          [ "$status" = 2 ] || fail "expected exit 2 for an argument, got $status: $output"

          # An event this cannot name a judged tree for builds, rather than
          # going looking for a verdict that was never taken on anything.
          answer="$( GITHUB_EVENT_NAME=schedule unbuilt-checks 2>/dev/null )" && status=0 || status=$?
          [ "$status" = 0 ] || fail "expected exit 0 for an unknown event, got $status"
          [ "$answer" = true ] ||
            fail "an unknown event answered '$answer' rather than asking for a build"

          touch $out
        '';

        # A release merge starts CI and Release on one commit, and both used to
        # build the same tree. Two verdicts on one tree is the defect: the
        # second build cannot say anything the first did not, and it can
        # disagree with it. So the release job reads CI's verdict instead of
        # taking one of its own, and this is the reading.
        #
        # It stands above an irreversible step, so it is written as a refusal
        # with one way through. Each function names what it found and only
        # "success" is a pass — a conclusion GitHub has not invented yet comes
        # back "refused" rather than falling through to green.
        #
        # A run's own conclusion is not enough on its own. GitHub concludes a
        # run "success" when its jobs were skipped, and ci.yml's
        # conventional-subject job is skipped on every push to main, so the job
        # that builds the tree is asked for by name. Renaming it in ci.yml
        # leaves this lookup empty and stops the release, which is the
        # direction for that mistake to fall.
        # The runs the gate will read a verdict out of, as a jq filter over what
        # `gh run list` answers. Shared so that the run a state is decided from
        # and the run whose jobs are then fetched cannot come apart.
        pushesToMain = ''[.[] | select(.event == "push" and .headBranch == "main")]'';

        ciVerdict = ''
          jq=${pkgs.jq}/bin/jq

          # What `gh run list --workflow ci.yml --commit <sha>` said, on stdin.
          #
          # Only a push to main is a verdict on the tree at that sha. A pull
          # request's run checks out the merge ref rather than the commit, so
          # it was never handed this tree, and `unbuilt-checks` may have let it
          # skip the build besides. Either way it concludes success while
          # proving nothing about a sha on main, and this is where a release
          # would otherwise read one.
          #
          # A push to main's run may skip that build too, where every
          # derivation this tree declares was already judged on the pull
          # request the squash is of. It is still a verdict on this tree.
          # `unbuilt-checks` reads the merge ref clause above as a condition
          # rather than an objection: it takes a pull request's green only
          # where that head contains this commit's parent, which is exactly
          # when the merge the run was handed was the head's own tree. What is
          # left is two equal drvPath sets, and those are the same build
          # rather than a similar one — so a green run says every check the
          # flake declares for this tree has passed, and not that this run is
          # where the building happened.
          #
          # A push to main makes one run, so several is a shape ci.yml cannot
          # produce. It is not a first-element pick, because choosing between
          # two verdicts is the guess this exists to refuse.
          ci_run_state() {
            $jq -r --arg sha "$1" '
              ${pushesToMain} as $push |
              if   length == 0 then "none"
              elif any(.[]; .headSha != $sha) then "stray"
              elif ($push | length) == 0 then "unusable"
              elif ($push | length) > 1 then "several"
              elif any($push[]; .status != "completed") then "running"
              elif all($push[]; .conclusion == "success") then "success"
              elif all($push[]; .conclusion == "cancelled") then "cancelled"
              else "refused" end'
          }

          ci_run_id() {
            $jq -r '${pushesToMain}[0].databaseId'
          }

          # What `gh api repos/{owner}/{repo}/actions/runs/<id>/jobs` said, on
          # stdin. The name is ci.yml's, and the two files have to agree.
          ci_check_job_state() {
            $jq -r '
              [.jobs[] | select(.name == "nix flake check")] as $job |
              if   ($job | length) == 0 then "missing"
              elif ($job | length) > 1 then "several"
              elif all($job[]; .conclusion == "success") then "success"
              else "refused" end'
          }
        '';

        # The release job calls this where it used to run `nix flake check`, so
        # a commit still reaches `cargo publish` only behind a full check of its
        # own tree. What changed is whose check it is.
        #
        # Waiting is the cost of keeping both workflows on the same push. The
        # cap is well inside the release job's own timeout-minutes, so a CI run
        # that never concludes ends here with a sentence rather than at the job
        # limit with none.
        awaitCiVerdict = pkgs.writeShellScriptBin "await-ci-verdict" ''
          set -u

          gh=${pkgs.gh}/bin/gh
          grep=${pkgs.gnugrep}/bin/grep
          date=${pkgs.coreutils}/bin/date
          sleep=${pkgs.coreutils}/bin/sleep

          ${ciVerdict}

          cap=2400
          interval=20

          if [ "$#" -ne 1 ]; then
            echo "usage: await-ci-verdict <full 40-character sha>" >&2
            echo >&2
            echo "Waits until CI has concluded for that commit, and exits 0 only for a" >&2
            echo "run whose own conclusion is success and whose nix flake check job" >&2
            echo "succeeded. Every other answer exits non-zero, silence included." >&2
            exit 2
          fi

          sha="$1"

          # gh answers a short sha with an empty list and exit 0, which is what
          # a commit with no run answers too. Nothing downstream can tell those
          # apart, so the shortening is refused here rather than read as a
          # commit CI has not reached yet.
          if ! printf '%s' "$sha" | $grep -Eq '^[0-9a-f]{40}$'; then
            echo "await-ci-verdict: '$sha' is not a full 40-character sha, and gh answers" >&2
            echo "a short one with an empty list and exit 0 — the same answer it gives for" >&2
            echo "a commit with no run. Pass GITHUB_SHA rather than an abbreviation." >&2
            exit 1
          fi

          deadline=$(( $($date +%s) + cap ))

          while :; do
            runs="$($gh run list --workflow ci.yml --commit "$sha" --limit 20 \
              --json databaseId,headSha,status,conclusion,event,headBranch)" || {
              echo "::error::gh would not list CI's runs for $sha, so this release has no verdict to publish on."
              exit 1
            }

            state="$(printf '%s' "$runs" | ci_run_state "$sha")" || state=unreadable

            case "$state" in
              success) break ;;
              none)    echo "no CI run for $sha yet" ;;
              running) echo "CI has not finished with $sha yet" ;;
              cancelled)
                echo "::error::CI's run for $sha was cancelled, so this commit has no verdict. A cancelled run is neither green nor red, and this release is not going out on one."
                exit 1
                ;;
              unusable)
                echo "::error::CI has run on $sha, but never as a push to main. A pull request's run builds the merge ref rather than this commit, and ci.yml lets it skip the build where its derivations match main's, so its success does not say this tree was checked. Release a commit that is on main."
                exit 1
                ;;
              stray)
                echo "::error::gh answered for a commit other than $sha. Refusing to guess which of those verdicts is this release's."
                exit 1
                ;;
              several)
                echo "::error::gh returned more than one CI run for $sha. ci.yml makes one run per push, so picking between them would be a guess about which tree was checked."
                exit 1
                ;;
              *)
                echo "::error::CI's run for $sha concluded something this cannot read as a pass."
                printf '%s\n' "$runs" >&2
                exit 1
                ;;
            esac

            if [ "$($date +%s)" -ge "$deadline" ]; then
              echo "::error::CI has still not concluded for $sha after $(( cap / 60 )) minutes. Nothing has been tagged or published. Once CI is green, dispatch this workflow with from=publish to release this commit."
              exit 1
            fi

            $sleep "$interval"
          done

          id="$(printf '%s' "$runs" | ci_run_id)"

          jobs="$($gh api "repos/{owner}/{repo}/actions/runs/$id/jobs")" || {
            echo "::error::CI's run $id passed for $sha, but GitHub would not say which of its jobs did. A run conclusion on its own does not say the tree was built, so this refuses rather than assume it."
            exit 1
          }

          job="$(printf '%s' "$jobs" | ci_check_job_state)" || job=unreadable

          case "$job" in
            success) ;;
            missing)
              echo "::error::CI's run $id for $sha has no job named 'nix flake check', so nothing in it says this tree was built. If that job was renamed in ci.yml, rename it in ciVerdict too."
              exit 1
              ;;
            *)
              echo "::error::CI's run $id passed for $sha, but its 'nix flake check' job did not. A run concludes success when its jobs are skipped, and a skipped check is not a checked tree."
              exit 1
              ;;
          esac

          echo "CI's run $id checked $sha and passed."
        '';

        # Nothing in this repository runs a workflow file, so the release's own
        # wiring is read rather than checked. What can be checked is the
        # decision, and this is where it is: every answer GitHub can give,
        # against the exit this repository wants for it.
        #
        # The two-job success fixture is the shape of a real push to main —
        # `nix flake check` succeeded and `conventional subject` was skipped —
        # which is why a run conclusion alone was not enough to read.
        awaitCiVerdictTest = pkgs.runCommand "await-ci-verdict-test"
          { nativeBuildInputs = [ awaitCiVerdict ]; } ''
          set -u
          ${ciVerdict}

          sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
          other=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb

          fail() { echo "FAIL: $1"; exit 1; }

          # A push to main, as gh reports one. `conclusion` is JSON, so a run
          # that has not finished is given the null gh actually sends.
          run() {
            printf '{"databaseId":1,"headSha":"%s","status":"%s","conclusion":%s,"event":"push","headBranch":"main"}' \
              "$1" "$2" "$3"
          }

          # The same commit's run as a pull request, which is the shape whose
          # success says least: it may have skipped the build, and what it
          # checked out was the merge ref.
          pull_request_run() {
            printf '{"databaseId":2,"headSha":"%s","status":"%s","conclusion":%s,"event":"pull_request","headBranch":"a-branch"}' \
              "$1" "$2" "$3"
          }

          reads() {
            want="$1"
            shift
            got="$(printf '[%s]' "$*" | ci_run_state "$sha")" || got="(jq refused it)"
            [ "$got" = "$want" ] ||
              fail "a run list this expected to read as '$want' read as '$got': [$*]"
          }

          reads none
          reads running   "$(run "$sha" in_progress null)"
          reads running   "$(run "$sha" queued null)"
          reads success   "$(run "$sha" completed '"success"')"
          reads cancelled "$(run "$sha" completed '"cancelled"')"

          # Every other conclusion GitHub records, and the one it records for a
          # run that finished without reaching one. None of these is a pass and
          # none of them is worth waiting on.
          reads refused "$(run "$sha" completed '"failure"')"
          reads refused "$(run "$sha" completed '"timed_out"')"
          reads refused "$(run "$sha" completed '"startup_failure"')"
          reads refused "$(run "$sha" completed '"action_required"')"
          reads refused "$(run "$sha" completed '"neutral"')"
          reads refused "$(run "$sha" completed '"stale"')"
          reads refused "$(run "$sha" completed '"skipped"')"
          reads refused "$(run "$sha" completed null)"

          # A conclusion nobody here has heard of is refused rather than
          # waited on or passed, so the day GitHub adds one no release goes out
          # on it.
          reads refused "$(run "$sha" completed '"embargoed"')"

          # The verdict belongs to another commit. A green one is still not
          # this release's, and gh answering with it at all is reason enough to
          # stop.
          reads stray "$(run "$other" completed '"success"')"
          reads stray "$(run "$sha" completed '"success"'), $(run "$other" completed '"success"')"

          # Two runs for the one commit is not a shape ci.yml makes, so it is
          # refused rather than resolved by taking the first.
          reads several \
            "$(run "$sha" completed '"success"'), $(run "$sha" completed '"failure"')"

          # Green, for this very commit, and still not a verdict on it. This is
          # what a release dispatched on a pull request's branch would find, and
          # taking it would publish a tree nothing had built.
          reads unusable "$(pull_request_run "$sha" completed '"success"')"
          reads unusable "$(pull_request_run "$sha" in_progress null)"

          # Where main's own run is there too, that one answers. A pull
          # request's run neither adds to it nor makes it ambiguous, so a
          # branch pushed at a commit that is already on main still releases.
          reads success \
            "$(run "$sha" completed '"success"'), $(pull_request_run "$sha" completed '"failure"')"

          # And the jobs are then fetched for main's run rather than for
          # whichever gh happened to list first.
          id="$(printf '[%s]' \
            "$(pull_request_run "$sha" completed '"failure"'), $(run "$sha" completed '"success"')" \
            | ci_run_id)"
          [ "$id" = 1 ] || fail "expected the push to main's run id, got '$id'"

          jobs() {
            got="$(printf '{"jobs":[%s]}' "$2" | ci_check_job_state)" || got="(jq refused it)"
            [ "$got" = "$1" ] ||
              fail "a job list this expected to read as '$1' read as '$got': $2"
          }

          check='{"name":"nix flake check","conclusion":"success"}'
          subject='{"name":"conventional subject","conclusion":"skipped"}'

          jobs success "$check,$subject"

          # The hole a run conclusion leaves. GitHub concludes the run success
          # in both of these, and in neither was the tree built.
          jobs refused '{"name":"nix flake check","conclusion":"skipped"},'"$subject"
          jobs refused '{"name":"nix flake check","conclusion":"failure"},'"$subject"

          # ci.yml renamed the job. The lookup empties and the release stops,
          # rather than reading a run with no check in it as a checked tree.
          jobs missing "$subject"
          jobs several "$check,$check"

          # The two the script decides on its own, before it has asked GitHub
          # anything.
          output="$( await-ci-verdict 2b487e6 2>&1 )" && status=0 || status=$?
          [ "$status" = 1 ] || fail "expected exit 1 for a short sha, got $status: $output"
          case "$output" in
            *"empty list"*) ;;
            *) fail "the short-sha refusal did not say what gh answers one with: $output" ;;
          esac

          output="$( await-ci-verdict 2>&1 )" && status=0 || status=$?
          [ "$status" = 2 ] || fail "expected exit 2 with no argument, got $status: $output"

          touch $out
        '';

        # A formula in homebrew-core is version-bumped by Homebrew's own bot,
        # which is the one service a tap does not come with. So the tap serves
        # whatever version was last written into it, and this is what writes the
        # next one: the release cuts the formula rather than a maintainer
        # editing a version and four checksums by hand and nothing noticing
        # when they do not.
        #
        # The checksums are read from the files the release downloaded beside
        # the binaries it is announcing, so nothing is hashed twice and nothing
        # is fetched in order to be hashed.
        tapFormula = pkgs.writeShellScriptBin "tap-formula" ''
          set -u

          grep=${pkgs.gnugrep}/bin/grep

          if [ "$#" -ne 2 ]; then
            echo "usage: tap-formula <version> <directory of release assets>" >&2
            echo >&2
            echo "Writes Formula/bdi.rb for that version to stdout, taking each" >&2
            echo "checksum from the bdi-<target>.sha256 beside its binary." >&2
            exit 2
          fi

          version="$1"
          assets="$2"

          # A checksum that could not be read would otherwise reach the formula
          # as the empty string, or as the one before it, and brew would report
          # a mismatch against a download nobody can reproduce. So each is read
          # into a variable of its own and asserted there, rather than
          # substituted where it is used.
          sum=""
          checksum() {
            file="$assets/bdi-$1.sha256"
            if [ ! -f "$file" ]; then
              echo "tap-formula: $file is not there, so this release has no checksum for $1." >&2
              exit 1
            fi
            sum=""
            read -r sum _ < "$file" || true
            if ! printf '%s' "$sum" | $grep -Eq '^[0-9a-f]{64}$'; then
              echo "tap-formula: $file does not open with a sha256: '$sum'" >&2
              exit 1
            fi
          }

          checksum aarch64-apple-darwin
          macos_arm="$sum"
          checksum x86_64-apple-darwin
          macos_intel="$sum"
          checksum aarch64-unknown-linux-musl
          linux_arm="$sum"
          checksum x86_64-unknown-linux-musl
          linux_intel="$sum"

          releases=https://github.com/CodeForBreakfast/beady-eye/releases/download

          cat <<EOF
          class Bdi < Formula
            desc "Tree of work in flight: bead graphs annotated with the live agents working them"
            homepage "https://github.com/CodeForBreakfast/beady-eye"
            license "Apache-2.0"

            on_macos do
              on_arm do
                url "$releases/v$version/bdi-aarch64-apple-darwin"
                sha256 "$macos_arm"
              end

              on_intel do
                url "$releases/v$version/bdi-x86_64-apple-darwin"
                sha256 "$macos_intel"
              end
            end

            on_linux do
              on_arm do
                url "$releases/v$version/bdi-aarch64-unknown-linux-musl"
                sha256 "$linux_arm"
              end

              on_intel do
                url "$releases/v$version/bdi-x86_64-unknown-linux-musl"
                sha256 "$linux_intel"
              end
            end

            def install
              bin.install Dir["bdi-*"].first => "bdi"
            end

            test do
              assert_match "bdi #{version}", shell_output("#{bin}/bdi --version")
            end
          end
          EOF
        '';

        # Nothing in this repository installs from the tap, and the formula is
        # read by brew rather than by anything here, so what a wrong one costs
        # is a reader's install rather than a red branch. This is what stands in
        # for that: the formula is asserted whole, against four checksums a
        # reader can tell apart on sight, so a pair swapped between two targets
        # is a failure here rather than a Mach-O binary offered to a Linux box.
        tapFormulaTest = pkgs.runCommand "tap-formula-test"
          { nativeBuildInputs = [ tapFormula ]; } ''
          set -u

          fail() { echo "FAIL: $1"; printf '%s\n' "$output"; exit 1; }
          output=""

          assets="$NIX_BUILD_TOP/assets"
          mkdir -p "$assets"
          checksum() {
            printf '%s  bdi-%s\n' "$2" "$1" > "$assets/bdi-$1.sha256"
          }

          checksum aarch64-apple-darwin      aaaaaaaa11111111aaaaaaaa11111111aaaaaaaa11111111aaaaaaaa11111111
          checksum x86_64-apple-darwin       bbbbbbbb22222222bbbbbbbb22222222bbbbbbbb22222222bbbbbbbb22222222
          checksum aarch64-unknown-linux-musl cccccccc33333333cccccccc33333333cccccccc33333333cccccccc33333333
          checksum x86_64-unknown-linux-musl dddddddd44444444dddddddd44444444dddddddd44444444dddddddd44444444

          tap-formula 9.9.9 "$assets" > "$NIX_BUILD_TOP/formula.rb" ||
            fail "it refused a complete set of assets:"

          cat > "$NIX_BUILD_TOP/expected.rb" <<'EOF'
          class Bdi < Formula
            desc "Tree of work in flight: bead graphs annotated with the live agents working them"
            homepage "https://github.com/CodeForBreakfast/beady-eye"
            license "Apache-2.0"

            on_macos do
              on_arm do
                url "https://github.com/CodeForBreakfast/beady-eye/releases/download/v9.9.9/bdi-aarch64-apple-darwin"
                sha256 "aaaaaaaa11111111aaaaaaaa11111111aaaaaaaa11111111aaaaaaaa11111111"
              end

              on_intel do
                url "https://github.com/CodeForBreakfast/beady-eye/releases/download/v9.9.9/bdi-x86_64-apple-darwin"
                sha256 "bbbbbbbb22222222bbbbbbbb22222222bbbbbbbb22222222bbbbbbbb22222222"
              end
            end

            on_linux do
              on_arm do
                url "https://github.com/CodeForBreakfast/beady-eye/releases/download/v9.9.9/bdi-aarch64-unknown-linux-musl"
                sha256 "cccccccc33333333cccccccc33333333cccccccc33333333cccccccc33333333"
              end

              on_intel do
                url "https://github.com/CodeForBreakfast/beady-eye/releases/download/v9.9.9/bdi-x86_64-unknown-linux-musl"
                sha256 "dddddddd44444444dddddddd44444444dddddddd44444444dddddddd44444444"
              end
            end

            def install
              bin.install Dir["bdi-*"].first => "bdi"
            end

            test do
              assert_match "bdi #{version}", shell_output("#{bin}/bdi --version")
            end
          end
          EOF

          output="$( diff -u "$NIX_BUILD_TOP/expected.rb" "$NIX_BUILD_TOP/formula.rb" 2>&1 )" ||
            fail "the formula is not the one this expects:"

          # An asset that never arrived, and one that arrived truncated. Both
          # have to be refused by name: a formula naming three checksums and one
          # blank is a file brew reads, and the release that wrote it has
          # already announced.
          rm "$assets/bdi-x86_64-apple-darwin.sha256"
          output="$( tap-formula 9.9.9 "$assets" 2>&1 )" && status=0 || status=$?
          [ "$status" = 1 ] || fail "expected exit 1 for a missing checksum, got $status:"
          case "$output" in
            *x86_64-apple-darwin*) ;;
            *) fail "the refusal did not name the target with no checksum:" ;;
          esac

          printf 'not a checksum\n' > "$assets/bdi-x86_64-apple-darwin.sha256"
          output="$( tap-formula 9.9.9 "$assets" 2>&1 )" && status=0 || status=$?
          [ "$status" = 1 ] || fail "expected exit 1 for a malformed checksum, got $status:"
          case "$output" in
            *sha256*) ;;
            *) fail "the refusal did not say what it read instead of a sha256:" ;;
          esac

          # A caller that passes only a version is asking for a formula built
          # from whatever is in the working directory. It is told instead.
          output="$( tap-formula 9.9.9 2>&1 )" && status=0 || status=$?
          [ "$status" = 2 ] || fail "expected exit 2 for a missing argument, got $status:"

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
          pkgs.cargo-mutants
          pkgs.watchexec
          rerunBdiOnChange
          checkBeforePush
          readCiVerdict
          conventionalSubject
          tapFormula
          mutationTestThisChange
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
        packages.await-ci-verdict = awaitCiVerdict;
        packages.conventional-subject = conventionalSubject;
        packages.tap-formula = tapFormula;
        packages.unbuilt-checks = unbuiltChecks;

        # `nix flake check` is the whole of CI. Anything CI should run belongs
        # here, not in the workflow that calls it.
        checks = {
          await-ci-verdict-test = awaitCiVerdictTest;
          build-and-test = beady-eye;
          check-before-push = checkBeforePushTest;
          clippy = checkOf "clippy" artifacts.dev [ pkgs.clippy ] "cargo clippy --all-targets -- -D warnings";
          conventional-subject-test = conventionalSubjectTest;

          # A second invocation rather than a flag on the one above, because
          # `--all-targets` is what defeats it: building the test targets pulls
          # in the dev-dependencies, which turns `testing` on, which makes the
          # library's modules `pub` again and switches `dead_code` off. Only a
          # build without them sees the narrow surface. See src/lib.rs.
          dead-code = checkOf "dead-code" artifacts.dev [ pkgs.clippy ] "cargo clippy -- -D warnings";
          documentation-is-not-source = documentationIsNotSource;
          fmt = checkOf "fmt" null [ pkgs.rustfmt ] "cargo fmt --check";
          limit-the-job-exceeded-test = limitTheJobExceededTest;
          module-concerns = checkOf "module-concerns" null [ modulesStateTheirConcern ]
            "modules-state-their-concern";
          module-concerns-test = modulesStateTheirConcernTest;
          palette = checkOf "palette" null [ coloursComeFromThePalette ]
            "colours-come-from-the-palette";
          palette-test = coloursComeFromThePaletteTest;

          # A source of its own, holding the two files this reads. It cannot be
          # a `checkOf` entry the way the scans above are: those are built from
          # `source`, which carries no documentation and
          # `documentation-is-not-source` is what keeps it that way.
          readme-pin = pkgs.runCommand "readme-pin"
            { nativeBuildInputs = [ readmePinsTheVersion ]; } ''
            readme-pins-the-version ${sourceOf [ ./README.md ./Cargo.toml ]}
            touch $out
          '';
          readme-pin-test = readmePinsTheVersionTest;

          # A source of its own for the same reason readme-pin has one: these
          # files are documentation, and `source` carries none.
          release-notes-wrap = pkgs.runCommand "release-notes-wrap"
            { nativeBuildInputs = [ releaseNotesAreUnwrapped ]; } ''
            release-notes-are-unwrapped ${sourceOf [ ./RELEASE-NOTES ]}
            touch $out
          '';
          release-notes-wrap-test = releaseNotesAreUnwrappedTest;

          refuse-a-run-that-scored-nothing-test = refuseARunThatScoredNothingTest;
          scope-to-the-change-test = scopeToTheChangeTest;
          name-the-runs-directory-test = nameTheRunsDirectoryTest;
          bound-the-machine-test = boundTheMachineTest;
          screen-walks = checkOf "screen-walks" null [ screenWalksAreBounded ]
            "screen-walks-are-bounded";
          screen-walks-test = screenWalksAreBoundedTest;
          tap-formula-test = tapFormulaTest;
          unbuilt-checks-test = unbuiltChecksTest;

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
          package = (checkOf "package" artifacts.dev [ ] ''
            set -o pipefail
            cargo package --offline --locked 2>&1 | tee package.log
            ! grep -qE "ignoring (library|binary) .* is not included" package.log
          '').overrideAttrs (_: { src = publishedSource; });
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

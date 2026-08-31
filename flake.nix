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
  };

  outputs = { self, nixpkgs, flake-utils, beads }:
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

      # The overlay and the per-system outputs are the same package, so a
      # consumer taking either gets what CI built.
      beadyEyeFor = pkgs: pkgs.rustPlatform.buildRustPackage {
        # The crate names the version once. A release tag that disagrees with it
        # is refused before anything is published, so a crate on crates.io
        # always has a flake output built from the same source at the same
        # version.
        pname = cargoToml.package.name;
        version = cargoToml.package.version;

        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;

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
      };
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        beady-eye = beadyEyeFor pkgs;

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

            not on main         CI runs on pushes to main. A commit anywhere
                                else has no run and will never get one.
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
              echo "NO RUN, AND NONE IS COMING — $sha is not on origin/main."
              echo "CI runs on pushes to main. Land it there and a run appears."
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
        # the build, so the two cannot drift apart.
        checkOf = name: tools: command:
          beady-eye.overrideAttrs (build: {
            pname = "${build.pname}-${name}";
            nativeBuildInputs = build.nativeBuildInputs ++ tools;
            buildPhase = command;
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
          clippy = checkOf "clippy" [ pkgs.clippy ] "cargo clippy --all-targets -- -D warnings";
          fmt = checkOf "fmt" [ pkgs.rustfmt ] "cargo fmt --check";

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
          package = checkOf "package" [ ] ''
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

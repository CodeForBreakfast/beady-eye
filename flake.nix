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

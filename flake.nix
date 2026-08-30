{
  description = "beady-eye - a bead graph annotated with live herdr pane activity";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Beads ships its own flake. Do not add inputs.nixpkgs.follows here — beads
    # needs Go 1.26 and this flake's nixpkgs carries an older toolchain.
    #
    # This tracker was created at 1.2.2 and is the only one on the shared server
    # at that version; the others are v53 stores created at 1.1.2. Moving this
    # pin moves the store, so treat it as a schema decision.
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
          buildInputs = [
            pkgs.git

            # Issue tracker. Pinned so every shell resolves the same binary as
            # the tracker's schema; the ambient bd on PATH is a different build.
            beads.packages.${system}.bd

            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
            pkgs.rust-analyzer
          ];

          shellHook = ''
            # Pin bd to this repo's .beads dir so it works from any
            # subdirectory. $PWD snapshots at shell entry (nix develop launches
            # in the project root), so this stays correct after cd elsewhere —
            # and stops a bare bd from inheriting another project's BEADS_DIR.
            export BEADS_DIR="$PWD/.beads"

            # Disable bd's smart remote-migrate gate. It is the only verdict
            # that can permit an in-place schema migration of a shared server,
            # and nothing here should ever migrate one — the tracker's server
            # is shared.
            export BD_SMART_GATE=0

            # The banner is diagnostic, so it goes where nix puts its own
            # diagnostics. On stdout it corrupts every `nix develop -c … --json`
            # a caller pipes into a parser.
            echo "👁  beady-eye Development Shell" >&2
            # Print where bd resolved from, not just what it claims to be — a
            # version alone cannot distinguish this shell's bd from PATH's.
            echo "beads: $(bd --version) ($(command -v bd))" >&2

            # BEADS_DOLT_PASSWORD lives here; the file is gitignored and 0600.
            if [ -f .env.local ]; then
              set -a
              source .env.local
              set +a
              echo "✅ Loaded environment from .env.local" >&2
            else
              echo "⚠️  no .env.local — bd cannot authenticate to tracker.example.invalid" >&2
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
          # It cannot catch a dropped crate root: cargo drops that with a
          # warning and an exit code of zero, then verifies an empty tarball.
          package = checkOf "package" [ ] ''
            set -o pipefail
            cargo package --offline --locked 2>&1 | tee package.log
            ! grep -q "is not included in the published package" package.log
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

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
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
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

            echo "👁  beady-eye Development Shell"
            # Print where bd resolved from, not just what it claims to be — a
            # version alone cannot distinguish this shell's bd from PATH's.
            echo "beads: $(bd --version) ($(command -v bd))"

            # BEADS_DOLT_PASSWORD lives here; the file is gitignored and 0600.
            if [ -f .env.local ]; then
              set -a
              source .env.local
              set +a
              echo "✅ Loaded environment from .env.local"
            else
              echo "⚠️  no .env.local — bd cannot authenticate to tracker.example.invalid"
            fi
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "beady-eye";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };
      }
    );
}

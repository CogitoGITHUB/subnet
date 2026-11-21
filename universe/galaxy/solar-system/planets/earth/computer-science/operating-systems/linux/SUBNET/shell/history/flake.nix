{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs =
    { self
    , nixpkgs
    , flake-utils
    , fenix
    , ...
    }:
    flake-utils.lib.eachDefaultSystem
      (system:
      let
        pkgs = nixpkgs.outputs.legacyPackages.${system};
      in
      {
        packages.shell-history = pkgs.callPackage ./shell-history.nix {
          rustPlatform =
            let
              toolchain =
                fenix.packages.${system}.fromToolchainFile
                  {
                    file = ./rust-toolchain.toml;
                    sha256 = "sha256-OATSZm98Es5kIFuqaba+UvkQtFsVgJEBMmS+t6od5/U=";
                  };
            in
            pkgs.makeRustPlatform {
              cargo = toolchain;
              rustc = toolchain;
            };
        };
        packages.default = self.outputs.packages.${system}.shell-history;

        devShells.default = self.packages.${system}.default.overrideAttrs (super: {
          nativeBuildInputs = with pkgs;
            super.nativeBuildInputs
            ++ [
              cargo-edit
              clippy
              rustfmt
            ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

          shellHook = ''
            echo >&2 "Setting development database path"
            export SUBNET_SHELL_HISTORY_DB_PATH="/tmp/shell_history_dev.db"
            export SUBNET_SHELL_HISTORY_RECORD_STORE_PATH="/tmp/shell_history_records.db"

            if [ -e "''${SUBNET_SHELL_HISTORY_DB_PATH}" ]; then
              echo >&2 "''${SUBNET_SHELL_HISTORY_DB_PATH} already exists, you might want to double-check that"
            fi

            if [ -e "''${SUBNET_SHELL_HISTORY_RECORD_STORE_PATH}" ]; then
              echo >&2 "''${SUBNET_SHELL_HISTORY_RECORD_STORE_PATH} already exists, you might want to double-check that"
            fi
          '';
        });
      })
    // {
      overlays.default = final: prev: {
        inherit (self.packages.${final.stdenv.hostPlatform.system}) shell-history;
      };
    };
}

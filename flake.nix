{
  description = "Priory runs coding agents on machines you own";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  inputs.rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

  outputs =
    { self, nixpkgs, rust-overlay }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
      priory =
        { lib, rustPlatform, sqlite }:
        rustPlatform.buildRustPackage {
          pname = "priory";
          inherit version;
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "herdr-bus-0.1.0" = "sha256-LpDkmcG54914PI1BkGwIbpkgLfK9gZ7T5XyCHmanCJ4=";
            };
          };
          buildInputs = [ sqlite ];
          cargoBuildFlags = [ "-p" "priory" ];
          # CI runs the test suite with tmux and git; the sandbox has neither.
          doCheck = false;
          meta = {
            description = "Runs coding agents on machines you own";
            homepage = "https://github.com/maxence-lefebvre/priory";
            license = lib.licenses.mit;
            mainProgram = "priory";
          };
        };
    in
    {
      overlays.default = final: _prev: { priory = final.callPackage priory { }; };

      packages = forAllSystems (pkgs: {
        default = pkgs.callPackage priory { };
      });

      # Makes `nix flake check` build the binary, so a Cargo.lock change that breaks it fails CI.
      checks = forAllSystems (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          meta.description = "The priory command";
        };
      });

      # The dev shell takes Rust from rust-toolchain.toml, as CI does, so clippy matches CI's.
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            ((pkgs.extend rust-overlay.overlays.default).rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            nodejs
            tmux
            git
          ];
        };
      });
    };
}

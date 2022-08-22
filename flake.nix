{
  inputs = {
      nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
      flake-utils.url = "github:numtide/flake-utils";
      naersk.url = "github:nix-community/naersk";
      rust-overlay.url = "github:oxalica/rust-overlay";
    };
  outputs = { self, nixpkgs, flake-utils, naersk, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustVersion = pkgs.rust-bin.stable.latest.default;
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustVersion;
          rustc = rustVersion;
        };

        naersk' = pkgs.callPackage naersk {};

        rustBuild = naersk'.buildPackage {
          src = ./.;
        };

        dockerImage = pkgs.dockerTools.buildImage {
          name = "rexml";
          config = { Cmd = [ "${rustBuild}/bin/rexml" ]; };
        };

      in {
        packages = {
            docker = dockerImage;
            rust = rustBuild;
        };
        defaultPackage = rustBuild;
        devShell = pkgs.mkShell {
          buildInputs =
            [ (rustVersion.override { extensions = [ "rust-src" ]; }) ] ++ (with pkgs; [
              rust-analyzer
              httpie
              sqlx-cli
              jq
              sqlite
            ]);
        };
      }
    );
}

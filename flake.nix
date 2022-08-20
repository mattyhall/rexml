{
  inputs = {
      nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
      flake-utils.url = "github:numtide/flake-utils";
      naersk.url = "github:nix-community/naersk";
    };
  outputs = { self, nixpkgs, flake-utils, naersk, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

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
            (with pkgs; [
              rust-analyzer
              httpie
              sqlx-cli
              jq
              sqlite
              httpie
            ]);
        };
      }
    );
}

{
  description = "Development environment with wrangler";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    wrangler = {
      url = "github:emrldnix/wrangler";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      wrangler,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        nix.settings = {
          substituters = [ "https://wrangler.cachix.org" ];
          trusted-public-keys = [ "wrangler.cachix.org-1:N/FIcG2qBQcolSpklb2IMDbsfjZKWg+ctxx0mSMXdSs=" ];
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            wrangler.packages.${system}.default
          ];
        };
      }
    );
}

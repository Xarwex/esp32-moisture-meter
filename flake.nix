{
  description = "ESP dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    esp32 = {
      url = "github:leighleighleigh/esp-rs-nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      perSystem =
        {
          pkgs,
          system,
          ...
        }:
        {
          devShells.default = pkgs.mkShell {
            packages = [
              # For copilot
              pkgs.bashInteractive

              inputs.esp32.packages.${system}.default
              pkgs.rust-analyzer
              pkgs.rustup
              pkgs.espflash
              pkgs.pkg-config
              pkgs.stdenv.cc
            ];

            shellHook = ''
              export RUSTUP_TOOLCHAIN=${inputs.esp32.packages.${system}.default}
            '';
          };
        };
    };
}

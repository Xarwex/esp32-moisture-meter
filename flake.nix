{
  description = "ESP dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    esp32 = {
      url = "github:leighleighleigh/esp-rs-nix/6be9cd080962efa09cac2bbf807efdedafa62269";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      esp32,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };
        espToolchain = esp32.packages.${system}.default;
        rustcWithSysroot = pkgs.writeShellScriptBin "rustc" ''
          exec "${espToolchain}/bin/rustc" --sysroot "${espToolchain}" "$@"
        '';
      in
      {
        devShells.default = pkgs.mkShell {
          # LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          RUSTUP_TOOLCHAIN = "${espToolchain}";
          RUST_SRC_PATH = "${espToolchain}/lib/rustlib/src/rust/library";
          RUSTC = "${rustcWithSysroot}/bin/rustc";
          # LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath [
          #   pkgs.zlib
          #   pkgs.stdenv.cc.cc.lib
          # ]}";

          inputsFrom = [
            esp32.devShells.${system}.default
          ];

          packages = [
            pkgs.rust-analyzer
          ];
        };
      }
    );
}

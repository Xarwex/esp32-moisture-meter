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
      in
      {
        devShells.default = pkgs.mkShell {
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          RUSTUP_TOOLCHAIN = "${esp32.packages.${system}.default}";
          RUST_SRC_PATH = "${esp32.packages.${system}.default}/lib/rustlib/src/rust/library";

          inputsFrom = [
            esp32.devShells.${system}.default
          ];

          buildInputs = [
            pkgs.rustup
          ];

          shellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.zlib ]}:$LD_LIBRARY_PATH"
            export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib.outPath}/lib:$LD_LIBRARY_PATH"
          '';

          packages = [
            pkgs.rustup
            pkgs.rust-analyzer
            # pkgs.clang
            #pkgs.rust-analyzer
            #esp-generate
            #cargo-feature
          ];
        };
      }
    );
}

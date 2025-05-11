{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        inherit (pkgs)
          fetchurl
          lib
          mkShellNoCC
          rust-bin
          stdenv
          ;

        # Rust toolchain for development
        rust-dev = (rust-bin.fromRustupToolchainFile ./arceos/rust-toolchain.toml).override {
          targets = [ "riscv64gc-unknown-none-elf" ];
        };
        rust-dev-with-rust-analyzer = rust-dev.override (prev: {
          extensions = prev.extensions ++ [
            "rust-src"
            "rust-analyzer"
          ];
        });

        # Use a prebuilt GNU RISC-V toolchain, since nixpkgs does not support riscv64-embedded.
        riscv-embedded-toolchain = lib.optionals (system == "x86_64-linux") (
          stdenv.mkDerivation rec {
            pname = "riscv-embedded-toolchain";
            version = "14.2.0-3";
            src = fetchurl {
              url = "https://github.com/xpack-dev-tools/riscv-none-elf-gcc-xpack/releases/download/v${version}/xpack-riscv-none-elf-gcc-${version}-linux-x64.tar.gz";
              hash = "sha256-9XRBW2PxKwm900dSI6tJKkZdI4EGRskME6TDtnbINQM=";
            };
            nativeBuildInputs = [ pkgs.autoPatchelfHook ];
            sourceRoot = "xpack-riscv-none-elf-gcc-${version}";
            installPhase = ''
              runHook preInstall
              cp -r . $out
              runHook postInstall
            '';
          }
        );
        riscv64-linux-musl-cross = stdenv.mkDerivation {
          pname = "riscv64-linux-musl-cross";
          version = "11.2.1";
          src = fetchurl {
            url = "https://musl.cc/riscv64-linux-musl-cross.tgz";
            hash = "sha256-2wvEE71Kk/IBLMdLm6DErynYvBi4jpxhmYc4zLkYYEs=";
          };
          nativeBuildInputs = [ pkgs.autoPatchelfHook ];
          sourceRoot = "riscv64-linux-musl-cross";
          installPhase = ''
            runHook preInstall
            cp -r . $out
            runHook postInstall
          '';
        };

        mkDevShell =
          devPkgs:
          (mkShellNoCC {
            packages =
              with pkgs;
              [
                cargo-binutils
                qemu
                tinyxxd
                riscv-embedded-toolchain
                riscv64-linux-musl-cross
              ]
              ++ devPkgs;
            shellHook = ''
              # Let makefiles choose rust-{objcopy,objdump}
              unset OBJCOPY
              unset OBJDUMP
              export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
            '';
          });
      in
      {
        packages.riscv64-linux-musl-cross = riscv64-linux-musl-cross;
        packages.riscv-embedded-toolchain = riscv-embedded-toolchain;

        # The default devShell with IDE integrations
        devShells.default = mkDevShell [ rust-dev-with-rust-analyzer ];
        # A minimal devShell without IDE integrations
        devShells.minimal = mkDevShell [ rust-dev ];
      }
    );
}

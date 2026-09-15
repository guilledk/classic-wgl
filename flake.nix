{
  description = "classic-wgl Rust port — cross-platform WebGL2 game engine";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rust-toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # Must match the `wasm-bindgen` crate version in Cargo.lock exactly:
        # `wasm-bindgen-test-runner` refuses test binaries built against another
        # version.
        wasm-bindgen-cli = pkgs.buildWasmBindgenCli rec {
          src = pkgs.fetchCrate {
            pname = "wasm-bindgen-cli";
            version = "0.2.127";
            hash = "sha256-di+qBAdd7pENLiIB9CoZoab+W5xeDoByMREcCGTSzWo=";
          };
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            inherit (src) pname version;
            hash = "sha256-FTv2GZIAQs0ePdIZXIXil7JbZ6kIT05VG6vqC1qNFxQ=";
          };
        };

        nativeBuildInputs = with pkgs; [
          pkg-config
          udev
          alsa-lib
          vulkan-loader
          libxkbcommon
          wayland
          mesa
          libGL
          libx11          # was xorg.libX11 (deprecated)
          libxcursor      # was xorg.libXcursor
          libxrandr       # was xorg.libXrandr
          libxi           # was xorg.libXi
        ];

        rustDeps = with pkgs; [
          rust-toolchain
          rust-analyzer
          wasm-bindgen-cli
          binaryen
          trunk
          lld
          emscripten
          # Headless browser for the wasm32 test harness (`wasm-bindgen-test`).
          chromium
          chromedriver
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = nativeBuildInputs ++ rustDeps;

          shellHook = ''
            if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
              echo "› Installing wasm32-unknown-unknown target…"
              rustup target add wasm32-unknown-unknown
            fi

            export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER="${pkgs.lld}/bin/lld"

            # `cargo test --target wasm32-unknown-unknown` runs the tests in
            # headless Chromium (cross-origin isolated, so SharedArrayBuffer works).
            export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="wasm-bindgen-test-runner"
            export CHROMEDRIVER="${pkgs.chromedriver}/bin/chromedriver"

            # X11/GL linking is handled by classic-platform's build.rs via
            # pkg-config (which Nix wraps to find libs from nativeBuildInputs).
            export RUSTFLAGS="-D warnings"
            export LD_LIBRARY_PATH="${pkgs.libxkbcommon}/lib:${pkgs.libx11}/lib:${pkgs.mesa}/lib:${pkgs.libGL}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

            echo "┌─────────────────────────────────────────┐"
            echo "│  classic-wgl Rust dev shell             │"
            echo "│  cargo xtask fetch-roms  fetch ROMs    │"
            echo "│  cargo run -p classic-desktop           │"
            echo "│  trunk serve              web dev       │"
            echo "│  cargo test                             │"
            echo "│  cargo clippy --all-targets             │"
            echo "│  cargo fmt --all -- --check             │"
            echo "└─────────────────────────────────────────┘"
          '';
        };
      }
    );
}

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

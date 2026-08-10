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

        # The set of native libs needed by winit + glutin on Linux
        nativeBuildInputs = with pkgs; [
          pkg-config
          udev
          alsa-lib
          vulkan-loader
          libxkbcommon
          wayland
          mesa
          libGL
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi
        ];

        # Rust toolchain with all components
        rustDeps = with pkgs; [
          rust-toolchain
          rust-analyzer
          wasm-bindgen-cli
          binaryen          # wasm-opt
          trunk             # wasm dev server + builder
          lld               # wasm32-unknown-unknown linker
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = nativeBuildInputs ++ rustDeps;

          shellHook = ''
            # Ensure wasm32 target is available
            if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
              echo "› Installing wasm32-unknown-unknown target…"
              rustup target add wasm32-unknown-unknown
            fi

            export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER="${pkgs.lld}/bin/lld"
            export RUSTFLAGS="-D warnings"

            echo "┌─────────────────────────────────────────┐"
            echo "│  classic-wgl Rust dev shell             │"
            echo "│  cargo build         native debug       │"
            echo "│  trunk build         web release        │"
            echo "│  trunk serve         web dev server     │"
            echo "│  cargo test          native tests       │"
            echo "│  cargo clippy        lint               │"
            echo "│  cargo fmt           format             │"
            echo "└─────────────────────────────────────────┘"
          '';
        };

        # Additional shell with Node for running the TS golden harness
        devShells.full = pkgs.mkShell {
          buildInputs = nativeBuildInputs ++ rustDeps ++ (with pkgs; [ nodejs_22 ]);
        };
      }
    );
}

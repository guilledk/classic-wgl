{
  description = "classic-wgl documentation tooling (d2 diagrams)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        # Interactive diagram work: `nix develop ./docs`, then `d2 ...`.
        devShells.default = pkgs.mkShell {
          packages = [ pkgs.d2 ];
          shellHook = ''
            echo "classic-wgl docs shell — render the architecture diagram with:"
            echo "  d2 architecture.d2 architecture.svg"
          '';
        };

        # Reproducible render: `nix build ./docs#architecture` (and the other
        # diagram packages) emit the SVG(s) into the Nix store.
        packages.architecture = pkgs.runCommand "classic-wgl-architecture"
          {
            src = ./.;
            nativeBuildInputs = [ pkgs.d2 ];
          } ''
          mkdir -p $out
          d2 "$src/architecture.d2" "$out/architecture.svg"
        '';
      });
}

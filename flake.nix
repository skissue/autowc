{
  description = "AutoWC";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = [
      "aarch64-linux"
      "x86_64-linux"
    ];

    forAllSystems = nixpkgs.lib.genAttrs systems;

    mkPackages = pkgs: let
      inherit (pkgs) lib rustPlatform;

      workspaceSrc = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.lock
          ./Cargo.toml
          ./autowc/Cargo.toml
          ./autowc/src
          ./autowc-mcp/Cargo.toml
          ./autowc-mcp/src
        ];
      };

      cargoLock = {
        lockFile = ./Cargo.lock;
      };

      common = {
        version = "0.1.0";
        src = workspaceSrc;
        inherit cargoLock;
      };

      autowc = rustPlatform.buildRustPackage (
        common
        // {
          pname = "autowc";
          cargoBuildFlags = [
            "-p"
            "autowc"
          ];
          cargoTestFlags = [
            "-p"
            "autowc"
          ];

          nativeBuildInputs = [
            pkgs.pkg-config
          ];

          buildInputs = [
            pkgs.libGL
            pkgs.libxkbcommon
            pkgs.wayland
          ];
        }
      );

      autowc-mcp = rustPlatform.buildRustPackage (
        common
        // {
          pname = "autowc-mcp";
          cargoBuildFlags = [
            "-p"
            "autowc-mcp"
          ];
          cargoTestFlags = [
            "-p"
            "autowc-mcp"
          ];

          nativeBuildInputs = [
            pkgs.makeWrapper
          ];

          postInstall = ''
            wrapProgram "$out/bin/autowc-mcp" \
              --prefix PATH : ${lib.makeBinPath [autowc]}
          '';
        }
      );
    in {
      inherit autowc autowc-mcp;
      default = autowc;
    };

    mkDevShell = pkgs:
      pkgs.mkShell {
        nativeBuildInputs = [
          pkgs.cargo
          pkgs.rustc
        ];

        buildInputs = [
          pkgs.libxkbcommon
          pkgs.rustfmt

          # Testing apps
          pkgs.foot
          pkgs.gtk4
          pkgs.wev
        ];

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
          pkgs.libGL
          pkgs.wayland
        ];
      };
  in {
    overlays.default = final: _prev: mkPackages final;

    packages = forAllSystems (
      system: let
        pkgs = import nixpkgs {inherit system;};
      in
        mkPackages pkgs
    );

    devShells = forAllSystems (
      system: let
        pkgs = import nixpkgs {inherit system;};
      in {
        default = mkDevShell pkgs;
      }
    );
  };
}

{pkgs ? import <nixpkgs> {}}:
with pkgs;
  mkShell {
    nativeBuildInputs = [rustc cargo];
    buildInputs = [
      libxkbcommon
      rustfmt

      # Testing apps
      foot
      gtk4
    ];
    LD_LIBRARY_PATH = lib.makeLibraryPath [wayland libGL];
  }

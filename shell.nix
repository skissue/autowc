{pkgs ? import <nixpkgs> {}}:
with pkgs;
  mkShell {
    nativeBuildInputs = [rustc cargo];
    buildInputs = [libxkbcommon rustfmt];
    LD_LIBRARY_PATH = lib.makeLibraryPath [wayland libGL];
  }

{pkgs ? import <nixpkgs> {}}:
pkgs.mkShell {
  packages = with pkgs; [
    pkg-config

    fontconfig
    gtk4
    blueprint-compiler
  ];
}

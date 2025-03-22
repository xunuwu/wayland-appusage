{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    crane,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {inherit system;};

      inherit (pkgs) lib;

      craneLib = crane.mkLib pkgs;

      src = lib.cleanSourceWith {
        src = ./.;
        filter = craneLib.filterCargoSources;
      };
    in {
      packages = {
        appusage-daemon = craneLib.buildPackage {
          pname = "appusage-daemon";
          inherit src;
          cargoExtraArgs = "-p appusage-daemon";
        };
        appusage = craneLib.buildPackage {
          pname = "appusage";
          inherit src;
          cargoExtraArgs = "-p appusage";
        };
      };
    });
}

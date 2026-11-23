{
  craneLib,
}:
let
  src = ../..;

  commonArgs = {
    inherit src;

    pname = "sisho-board";
    version = "0.1.0";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    cargoExtraArgs = "-p sisho-board";

    doCheck = true;
  }
)

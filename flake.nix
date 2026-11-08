{
  description = "opal: UI craft skill and ui-probe";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          ui-probe = pkgs.rustPlatform.buildRustPackage {
            pname = "ui-probe";
            version = "0.1.0";

            src = pkgs.lib.cleanSourceWith {
              src = ./ui-probe;
              filter = path: _type: builtins.baseNameOf path != "target";
            };

            cargoLock.lockFile = ./ui-probe/Cargo.lock;

            nativeBuildInputs = [
              pkgs.makeWrapper
              pkgs.rustfmt
            ];

            postInstall = ''
              wrapProgram $out/bin/ui-probe --set UI_PROBE_CHROME ${pkgs.chromium}/bin/chromium
            '';

            meta = {
              description = "Render a page in headless Chromium and emit UI evidence as JSON";
              license = pkgs.lib.licenses.mit;
              mainProgram = "ui-probe";
            };
          };
        in
        {
          inherit ui-probe;
          default = ui-probe;
          rules = pkgs.writeText "opal-rules.json" (
            builtins.readFile ./skills/opal/references/impeccable-rules.json
          );
        }
      );

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.system}.ui-probe ];
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
            pkgs.rust-analyzer
            pkgs.chromium
          ];
        };
      });
    };
}

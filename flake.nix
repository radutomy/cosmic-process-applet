{
  description = "A COSMIC applet for monitoring and killing memory-hungry user workloads";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          package = pkgs.rustPlatform.buildRustPackage {
            pname = "cosmic-process-applet";
            version = "0.1.0";
            src = self;

            cargoHash = "sha256-Va25WkX1YTuofx5yooR55n/08uCx2LxCSX2DaQRWRgU=";

            nativeBuildInputs = with pkgs; [
              just
              libcosmicAppHook
            ];

            dontUseJustBuild = true;
            dontUseJustCheck = true;
            justFlags = [
              "--set"
              "prefix"
              (placeholder "out")
              "--set"
              "cargo-target-dir"
              "target/${pkgs.stdenv.hostPlatform.rust.cargoShortTarget}"
            ];

            meta = {
              description = "COSMIC applet for killing memory-hungry user workloads";
              homepage = "https://github.com/radutomy/cosmic-process-applet";
              license = pkgs.lib.licenses.mpl20;
              mainProgram = "cosmic-process-applet";
              platforms = pkgs.lib.platforms.linux;
            };
          };
        in
        {
          cosmic-process-applet = package;
          default = package;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/cosmic-process-applet";
          meta.description = "Run the COSMIC process applet";
        };
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
            packages = with pkgs; [
              cargo
              clippy
              rust-analyzer
              rustc
              rustfmt
            ];
          };
        }
      );

      overlays.default = final: _: {
        cosmic-process-applet = self.packages.${final.stdenv.hostPlatform.system}.default;
      };
    };
}

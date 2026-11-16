{
  description = "q, a fast implementation of quilt";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      q =
        {
          lib,
          stdenv,
          rustPlatform,
          git,
          diffutils,
          gnupatch,
          diffstat,
          util-linux,
        }:
        rustPlatform.buildRustPackage {
          pname = "q";
          version = self.shortRev or self.dirtyShortRev or "unknown";
          src = lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          env.Q_VERSION = self.shortRev or self.dirtyShortRev or "unknown";

          # Tests drive q against real git, diff, patch and diffstat.
          nativeCheckInputs = [
            git
            diffutils
            gnupatch
            diffstat
          ]
          ++ lib.optional stdenv.hostPlatform.isLinux util-linux;

          # Needs util-linux `script -c`, which macOS's script lacks.
          checkFlags = lib.optionals stdenv.hostPlatform.isDarwin [
            "--skip=paged_commands_honor_quilt_pager_on_tty"
          ];

          meta = {
            description = "Fast implementation of quilt";
            homepage = "https://github.com/imputnet/q";
            license = lib.licenses.gpl3Plus;
            mainProgram = "q";
          };
        };
    in
    {
      packages = forAllSystems (pkgs: {
        q = pkgs.callPackage q { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.q;
      });

      overlays.default = final: _: { q = final.callPackage q { }; };

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}

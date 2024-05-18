{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { nixpkgs, ... }:
  with nixpkgs.lib;
  rec {
    mkPackages = pkgs:
    {
      default = pkgs.callPackage (
        {rustPlatform, openssl, pkg-config}:

        rustPlatform.buildRustPackage {
          name = "modem-exporter";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          buildInputs = [ openssl ];
          nativeBuildInputs = [ pkg-config ];
        }
      ) {};
    };

    pkgsAarch64 = mkPackages (import nixpkgs {
      localSystem = "x86_64-linux";
      crossSystem = "aarch64-linux";
    });

    nixosModules.default = { config, pkgs, ... }:
    let
      svcConfig = config.services.modem-exporter;
      packages = mkPackages pkgs;
    in {
      options = {
        services.modem-exporter = {
          enable = mkOption {
            type = types.bool;
            default = true;
          };
          environment = mkOption {
            type = types.attrsOf types.anything;
            default = {};
          };
          environmentFile = mkOption {
            type = types.nullOr types.path;
            default = null;
          };
        };
      };

      config = mkIf svcConfig.enable {
        systemd.services.modem-exporter = {
          wants = ["network-online.target"];
          after = ["network-online.target"];
          wantedBy = ["multi-user.target"];

          environment = svcConfig.environment;

          serviceConfig = mkMerge [
            {
              ExecStart = "${packages.default}/bin/modem-exporter";
              Restart = "on-failure";

              DynamicUser = true;
              CapabilityBoundingSet = "";
              LockPersonality = true;
              NoNewPrivileges = true;
              PrivateDevices = true;
              PrivateMounts = true;
              PrivateTmp = true;
              PrivateUsers = true;
              ProtectClock = true;
              ProtectControlGroups = true;
              ProtectHome = true;
              ProtectHostname = true;
              ProtectKernelLogs = true;
              ProtectKernelModules = true;
              ProtectKernelTunables = true;
              ProtectProc = "invisible";
              ProtectSystem = "strict";
              RemoveIPC = true;
              RestrictNamespaces = true;
              RestrictRealtime = true;
              RestrictSUIDSGID = true;
              SystemCallArchitectures = "native";
              SystemCallFilter = "@system-service";
            }
            (mkIf (svcConfig.environmentFile != null) {
              EnvironmentFile = svcConfig.environmentFile;
            })
          ];
        };
      };
    };
  };
}

{
  description = "death";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    crate2nix.url = "github:nix-community/crate2nix";
    crate2nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      crate2nix,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system}.default =
        (crate2nix.tools.${system}.appliedCargoNix {
          name = "router";
          src = ./.;
        }).rootCrate.build;
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          pkgs.rustfmt
          pkgs.cargo
          pkgs.clippy
          pkgs.rustc
        ];
        buildInputs = self.packages.${system}.default.buildInputs;
      };
      checks.${system} = {
        dhcp-tests = pkgs.testers.runNixOSTest {
          name = "try-dhcpv6";
          # shamelessly stolen from: https://github.com/NixOS/nixpkgs/blob/990146f6a35a2f578591e84d72329ee5db20280f/nixos/tests/systemd-networkd-ipv6-prefix-delegation.nix#L22C1-L179C11

          # The ISP's routers job is to delegate IPv6 prefixes via DHCPv6. Like with
          # regular IPv6 auto-configuration it will also emit IPv6 router
          # advertisements (RAs). Those RA's will not carry a prefix but in contrast
          # just set the "Other" flag to indicate to the receiving nodes that they
          # should attempt DHCPv6.
          #
          # Note: On the ISPs device we don't really care if we are using networkd in
          # this example. That being said we can't use it (yet) as networkd doesn't
          # implement the serving side of DHCPv6. We will use ISC Kea for that task.
          nodes.isp =
            { lib, pkgs, ... }:
            {
              virtualisation.vlans = [ 1 ];
              networking = {
                useDHCP = false;
                firewall.enable = false;
                interfaces.eth1 = lib.mkForce { };
              };

              systemd.network = {
                enable = true;

                networks = {
                  "eth1" = {
                    matchConfig.Name = "eth1";
                    address = [
                      "2001:DB8::1/64"
                    ];
                    networkConfig.IPv4Forwarding = true;
                    networkConfig.IPv6Forwarding = true;
                  };
                };
              };

              # Since we want to program the routes that we delegate to the "customer"
              # into our routing table we must provide kea with the required capability.
              systemd.services.kea-dhcp6-server.serviceConfig = {
                AmbientCapabilities = [ "CAP_NET_ADMIN" ];
                CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
              };

              services = {
                # Configure the DHCPv6 server to hand out both IA_NA and IA_PD.
                #
                # We will hand out /48 prefixes from the subnet 2001:DB8:F000::/36.
                # That gives us ~8k prefixes. That should be enough for this test.
                #
                # Since (usually) you will not receive a prefix with the router
                # advertisements we also hand out /128 leases from the range
                # 2001:DB8:0000:0000:FFFF::/112.
                kea.dhcp6 = {
                  enable = true;
                  settings = {
                    interfaces-config.interfaces = [ "eth1" ];
                    subnet6 = [
                      {
                        id = 1;
                        interface = "eth1";
                        subnet = "2001:DB8::/32";
                        rapid-commit = true;
                        pd-pools = [
                          {
                            prefix = "2001:DB8:1000::";
                            prefix-len = 36;
                            delegated-len = 48;
                          }
                        ];
                        pools = [
                          {
                            pool = "2001:DB8:0000:0000::-2001:DB8:0FFF:FFFF::FFFF";
                          }
                        ];
                      }
                    ];

                    # This is the glue between Kea and the Kernel FIB. DHCPv6
                    # rightfully has no concept of setting up a route in your
                    # FIB. This step really depends on your setup.
                    #
                    # In a production environment your DHCPv6 server is likely
                    # not the router. You might want to consider BGP, NETCONF
                    # calls, … in those cases.
                    #
                    # In this example we use the run script hook, that lets use
                    # execute anything and passes information via the environment.
                    # https://kea.readthedocs.io/en/kea-2.2.0/arm/hooks.html#run-script-run-script-support-for-external-hook-scripts
                    hooks-libraries = [
                      {
                        library = "${pkgs.kea}/lib/kea/hooks/libdhcp_run_script.so";
                        parameters = {
                          name = pkgs.writeShellScript "kea-run-hooks" ''
                            export PATH="${
                              lib.makeBinPath (
                                with pkgs;
                                [
                                  coreutils
                                  iproute2
                                ]
                              )
                            }"

                            set -euxo pipefail

                            leases6_committed() {
                              for i in $(seq $LEASES6_SIZE); do
                                idx=$((i-1))
                                prefix_var="LEASES6_AT''${idx}_ADDRESS"
                                plen_var="LEASES6_AT''${idx}_PREFIX_LEN"

                                ip -6 route replace ''${!prefix_var}/''${!plen_var} via $QUERY6_REMOTE_ADDR dev $QUERY6_IFACE_NAME
                              done
                            }

                            unknown_handler() {
                              echo "Unhandled function call ''${*}"
                              exit 123
                            }

                            case "$1" in
                                "leases6_committed")
                                    leases6_committed
                                    ;;
                                *)
                                    unknown_handler "''${@}"
                                    ;;
                            esac
                          '';
                          sync = false;
                        };
                      }
                    ];
                  };
                };

                # Finally we have to set up the router advertisements. While we could be
                # using networkd or bird for this task `radvd` is probably the most
                # venerable of them all. It was made explicitly for this purpose and
                # the configuration is much more straightforward than what networkd
                # requires.
                # As outlined above we will have to set the `Managed` flag as otherwise
                # the clients will not know if they should do DHCPv6. (Some do
                # anyway/always)
                radvd = {
                  enable = true;
                  config = ''
                    interface eth1 {
                      AdvSendAdvert on;
                      AdvManagedFlag on;
                      AdvOtherConfigFlag off; # we don't really have DNS or NTP or anything like that to distribute
                      prefix ::/64 {
                        AdvOnLink on;
                        AdvAutonomous on;
                      };
                    };
                  '';
                };

              };
            };

          nodes.client =
            { pkgs, lib, ... }:
            {
              virtualisation.vlans = [
                1
              ];
              boot.kernel.sysctl = {
                "net.ipv6.conf.all.forwarding" = 1;
              };

              networking = {
                useDHCP = false;
                firewall.enable = false;
                dhcpcd.enable = lib.mkForce false;
                interfaces = lib.mkForce { };
              };
              systemd.services.dhcp = {
                enable = true;
                after = [ "network.target" ];
                wants = [ "network.target" ];
                wantedBy = [
                  # "network-online.target"
                  # "multi-user.target"
                ];
                before = [ "network-online.target" ];
                serviceConfig.ExecStart = "${self.packages.${system}.default}/bin/router";
                environment = {
                  RUST_BACKTRACE = "1";
                };
              };
              systemd.network = {
                enable = true;

                networks = {
                  "eth1" = {
                    matchConfig.Name = "eth1";
                    networkConfig.DHCP = false;
                    ipv6AcceptRAConfig.DHCPv6Client = false;
                  };
                };
              };
              environment.systemPackages = [
                self.packages.${system}.default
              ];
            };
          interactive.sshBackdoor.enable = true;
          testScript = ''
            import time

            isp.start()
            client.start()


            # wait for the DHCP stuff on the router to come up.
            isp.wait_for_unit("kea-dhcp6-server.service")
            time.sleep(1)

            client.succeed("systemctl start dhcp.service")
            time.sleep(1)

            print(isp.succeed("journalctl -u kea-dhcp6-server.service --no-pager"))
            print("KEA LOGS DONE...")
            print(client.succeed("journalctl -u dhcp.service --no-pager"))
            print("DHCP LOGS DONE...")
            print(client.succeed("ip -6 a"))
          '';
        };
      };
    };
}

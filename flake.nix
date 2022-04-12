{
  description = "A modern, simple TCP tunnel in Rust that exposes local ports to a remote server, bypassing standard NAT connection firewalls.";
  inputs.nixCargoIntegration.url = "github:yusdacra/nix-cargo-integration";
  outputs = inputs: inputs.nixCargoIntegration.lib.makeOutputs { root = ./.; defaultOutputs.app = "bore"; };
}

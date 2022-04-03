# bore

A modern, simple TCP tunnel in Rust that exposes local ports to a self-hosted remote server, bypassing standard NAT connection firewalls. **That's all it does: no more, and no less.**

```shell
# Step 1: Installation (requires Rust)
cargo install bore-cli

# Step 2: On a remote server at example.com
bore proxy

# Step 3: On your local machine
bore local 8000 --to example.com:9000
```

This will expose your local port at `localhost:8000` to the public internet at `example.com:9000`.

Inspired by [localtunnel](https://github.com/localtunnel/localtunnel) and [ngrok](https://ngrok.io/), except `bore` is intended to be a highly efficient, unopinionated tool for real production workloads that is simple to install and use, with no frills attached.

## Detailed Usage

TODO

## Protocol

There is an implicit _control port_ at `7835`, used for creating new connections on demand. This can be configured in the command-line options.

## Acknowledgements

Created by Eric Zhang ([@ekzhang1](https://twitter.com/ekzhang1)). Licensed under the [MIT license](LICENSE).

The author would like to thank the contributors and maintainers of the [Tokio](https://tokio.rs/) project for making it possible to write ergonomic and efficient network services in Rust.

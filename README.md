# bore

[![Build status](https://img.shields.io/github/actions/workflow/status/ekzhang/bore/ci.yml)](https://github.com/ekzhang/bore/actions)
[![Crates.io](https://img.shields.io/crates/v/bore-cli.svg)](https://crates.io/crates/bore-cli)

A modern, simple TCP tunnel in Rust that exposes local ports to a remote server, bypassing standard NAT connection firewalls. **That's all it does: no more, and no less.**

![Video demo](https://i.imgur.com/vDeGsmx.gif)

```shell
# Installation (requires Rust, see alternatives below)
cargo install bore-cli

# On your local machine
bore local 8000 --to bore.pub
```

This will expose your local port at `localhost:8000` to the public internet at `bore.pub:<PORT>`, where the port number is assigned randomly.

Similar to [localtunnel](https://github.com/localtunnel/localtunnel) and [ngrok](https://ngrok.io/), except `bore` is intended to be a highly efficient, unopinionated tool for forwarding TCP traffic that is simple to install and easy to self-host, with no frills attached.

(`bore` totals about 400 lines of safe, async Rust code and is trivial to set up — just run a single binary for the client and server.)

## Installation

If you're on macOS, `bore` is packaged as a Homebrew core formula.

```shell
brew install bore-cli
```

Otherwise, the easiest way to install bore is from prebuilt binaries. These are available on the [releases page](https://github.com/ekzhang/bore/releases) for macOS, Windows, and Linux. Just unzip the appropriate file for your platform and move the `bore` executable into a folder on your PATH.

You also can build `bore` from source using [Cargo](https://doc.rust-lang.org/cargo/), the Rust package manager. This command installs the `bore` binary at a user-accessible path.

```shell
cargo install bore-cli
```

We also publish versioned Docker images for each release. The image is built for an AMD 64-bit architecture. They're tagged with the specific version and allow you to run the statically-linked `bore` binary from a minimal "scratch" container.

```shell
docker run -it --init --rm --network host ekzhang/bore <ARGS>
```

## Detailed Usage

This section describes detailed usage for the `bore` CLI command.

### Local Forwarding

You can forward a port on your local machine by using the `bore local` command. This takes a positional argument, the local port to forward, as well as a mandatory `--to` option, which specifies the address of the remote server.

```shell
bore local 5000 --to bore.pub
```

You can optionally pass in a `--port` option to pick a specific port on the remote to expose, although the command will fail if this port is not available. Also, passing `--local-host` allows you to expose a different host on your local area network besides the loopback address `localhost`.

The full options are shown below.

```shell
Starts a local proxy to the remote server

Usage: bore local [OPTIONS] --to <TO> <LOCAL_PORT>

Arguments:
  <LOCAL_PORT>  The local port to expose

Options:
  -l, --local-host <HOST>  The local host to expose [default: localhost]
  -t, --to <TO>            Address of the remote server to expose local ports to [env: BORE_SERVER=]
  -p, --port <PORT>        Optional port on the remote server to select [default: 0]
  -s, --secret <SECRET>    Optional secret for authentication [env: BORE_SECRET]
  -h, --help               Print help information
```

### Self-Hosting

As mentioned in the startup instructions, there is a public instance of the `bore` server running at `bore.pub`. However, if you want to self-host `bore` on your own network, you can do so with the following command:

```shell
bore server
```

That's all it takes! After the server starts running at a given address, you can then update the `bore local` command with option `--to <ADDRESS>` to forward a local port to this remote server.

The full options for the `bore server` command are shown below.

```shell
Runs the remote proxy server

Usage: bore server [OPTIONS]

Options:
      --min-port <MIN_PORT>  Minimum accepted TCP port number [default: 1024]
      --max-port <MAX_PORT>  Maximum accepted TCP port number [default: 65535]
  -s, --secret <SECRET>      Optional secret for authentication [env: BORE_SECRET]
  -h, --help                 Print help information
```

## Protocol

There is an implicit _control port_ at `7835`, used for creating new connections on demand. At initialization, the client sends a "Hello" message to the server on the TCP control port, asking to proxy a selected remote port. The server then responds with an acknowledgement and begins listening for external TCP connections.

Whenever the server obtains a connection on the remote port, it generates a secure [UUID](https://en.wikipedia.org/wiki/Universally_unique_identifier) for that connection and sends it back to the client. The client then opens a separate TCP stream to the server and sends an "Accept" message containing the UUID on that stream. The server then proxies the two connections between each other.

For correctness reasons and to avoid memory leaks, incoming connections are only stored by the server for up to 10 seconds before being discarded if the client does not accept them.

## Authentication

On a custom deployment of `bore server`, you can optionally require a _secret_ to prevent the server from being used by others. The protocol requires clients to verify possession of the secret on each TCP connection by answering random challenges in the form of HMAC codes. (This secret is only used for the initial handshake, and no further traffic is encrypted by default.)

```shell
# on the server
bore server --secret my_secret_string

# on the client
bore local <LOCAL_PORT> --to <TO> --secret my_secret_string
```

If a secret is not present in the arguments, `bore` will also attempt to read from the `BORE_SECRET` environment variable.

## Acknowledgements

Created by Eric Zhang ([@ekzhang1](https://twitter.com/ekzhang1)). Licensed under the [MIT license](LICENSE).

The author would like to thank the contributors and maintainers of the [Tokio](https://tokio.rs/) project for making it possible to write ergonomic and efficient network services in Rust.

# bore

[![Build status](https://img.shields.io/github/workflow/status/ekzhang/bore/CI)](https://github.com/ekzhang/bore/actions)
[![Crates.io](https://img.shields.io/crates/v/bore-cli.svg)](https://crates.io/crates/bore-cli)

A modern, simple TCP tunnel in Rust that exposes local ports to a remote server, bypassing standard NAT connection firewalls. **That's all it does: no more, and no less.**

![Video demo](https://i.imgur.com/vDeGsmx.gif)

```shell
# Installation (requires Rust)
cargo install bore-cli

# On your local machine
bore local 8000 --to bore.pub
```

This will expose your local port at `localhost:8000` to the public internet at `bore.pub:<PORT>`, where the port number is assigned randomly.

Similar to [localtunnel](https://github.com/localtunnel/localtunnel) and [ngrok](https://ngrok.io/), except `bore` is intended to be a highly efficient, unopinionated tool for forwarding TCP traffic that is simple to install and easy to self-host, with no frills attached.

(`bore` totals less than 400 lines of safe, async Rust code and is trivial to set up — just run a single binary for the client and server.)

## Detailed Usage

This section describes detailed usage for the `bore` CLI command.

### Local Forwarding

You can forward a port on your local machine by using the `bore local` command. This takes a positional argument, the local port to forward, as well as a mandatory `--to` option, which specifies the address of the remote server.

```shell
bore local 5000 --to bore.pub
```

You can optionally pass in a `--port` option to pick a specific port on the remote to expose, although the command will fail if this port is not available.

The full options are shown below.

```shell
bore-local 0.2.1
Starts a local proxy to the remote server

USAGE:
    bore local [OPTIONS] --to <TO> <LOCAL_PORT>

ARGS:
    <LOCAL_PORT>    The local port to listen on

OPTIONS:
    -h, --help               Print help information
    -p, --port <PORT>        Optional port on the remote server to select [default: 0]
    -s, --secret <SECRET>    Optional secret for authentication
    -t, --to <TO>            Address of the remote server to expose local ports to
    -V, --version            Print version information
```

### Self-Hosting

As mentioned in the startup instructions, there is an public instance of the `bore` server running at `bore.pub`. However, if you want to self-host `bore` on your own network, you can do so with the following command:

```shell
bore server
```

That's all it takes! After the server starts running at a given address, you can then update the `bore local` command with option `--to <ADDRESS>` to forward a local port to this remote server.

The full options for the `bore server` command are shown below.

```shell
bore-server 0.2.1
Runs the remote proxy server

USAGE:
    bore server [OPTIONS]

OPTIONS:
    -h, --help                   Print help information
        --min-port <MIN_PORT>    Minimum TCP port number to accept [default: 1024]
    -s, --secret <SECRET>        Optional secret for authentication
    -V, --version                Print version information
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

## Acknowledgements

Created by Eric Zhang ([@ekzhang1](https://twitter.com/ekzhang1)). Licensed under the [MIT license](LICENSE).

The author would like to thank the contributors and maintainers of the [Tokio](https://tokio.rs/) project for making it possible to write ergonomic and efficient network services in Rust.

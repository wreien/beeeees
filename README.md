# Bees Game

A coöperative multiplayer network-based game,
where players must control swarms of bees
to collect as much pollen as possible.

Developed for Reboot 2021.

**Note:** this is currently in very early development,
and still needs a lot of work to be functional.

## Getting Started

To compile the server, you will need to download
the [Rust](https://www.rust-lang.org/) programming language.
I recommend using [rustup](https://rustup.rs/).

Once you have installed Rust, open a new terminal
wherever you cloned this repository.
Use the following command to build and run the server on your machine.
This should install all required dependencies for you.
```sh
cargo run
```

When running, the server hosts a very simple website frontend on your machine.
By default you may access it by navigating to http://127.0.0.1:8080/.

There is also a very rudimentary "echo" client
you can use to interact with the server.
You can run it using:
```sh
cargo run --bin echo
```

The above will run in debug mode; 
you may additionally pass `--release` to enable compiler optimisations.

**TODO:** proper client programs (in python?)

## Documentation

You can generate documentation describing the internals of the server with:

```sh
cargo doc --open
```

The communication protocol is described [here](protocol.md).
However, for more details it is probably better
to read the documentation for the server itself.

## Logging

By default the server logs a number of interesting events to `stderr`.
You can customise the logging using the `RUST_LOG` environment variable.
For example, on Unix-like systems you might do the following:

```sh
env RUST_LOG=trace cargo run    # log everything
# or
env RUST_LOG=warn cargo run     # only log warnings and error messages
```

See [the `env_logger` documentation](https://docs.rs/env_logger)
for more details.

## Configuration

Currently the server allows some very simple configuration
via command-line parameters. 
You may explore the available options using
```sh
cargo run -- --help
```

Note the initial `--` required to separate 
arguments to `cargo` from arguments to the server.
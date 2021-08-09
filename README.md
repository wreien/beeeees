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
cargo run --release
```

There is also a very rudimentary "echo" client
you can use to interact with the server.
You can run it using:
```sh
cargo run --bin echo --release
```

**TODO:** proper client programs (in python?)
and website frontend (just a static JS thing, hopefully, though maybe node?).

## Documentation

You can generate documentation describing the internals of the server with:

```sh
cargo doc --open
```

The communication protocol is described [here](protocol.md).
However, for more details it is probably better
to read the documentation for the server itself.
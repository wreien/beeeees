# Bees Game: Clients

This directory contains a number of client programs
that can be used to interface with the server and play the game.

## Running the examples

All client programs are written using [Python](https://www.python.org/).
Visit the [downloads page](https://www.python.org/downloads/)
and install the version appropriate for your operating system.
Make sure you have at least **Python 3.7**.

_Note: currently only tested with Python 3.9..._

The current available examples are:

| Example        | Description                                   |
| -------------- | --------------------------------------------- |
| [eg_random][1] | Bees walk completely randomly around the map. |

[1]: eg_random.py

## Writing your own clients

You can write your client in any language,
you just need to be able to communicate with the server
(see the [protocol description](../protocol.md)).

The easiest way, however, is to use Python like the examples here.
Most of the marshalling work has been done for you by
[the "beeees" module](beeees.py),
which defines a number of helpful types and wrappers
to communicate with the server.
See some of the examples for how to use it.

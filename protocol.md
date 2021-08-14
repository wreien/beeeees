# Protocol

Currently the server is hardcoded to start
at the address `127.0.0.1:49998`, using a TCP stream.
All transmission is done using UTF-8 encoded
[JSON](https://www.json.org/json-en.html),
delimited by newlines for easy parsing.

The server will send messages to the client,
notifying events such as registration information, updates, and errors.
The client in return send a message
with handshaking information and actions they perform.

In this documentation all JSON examples have been "prettified"
for easier reading and understanding.
However, in practice there should be no newlines in a message,
either sending or receiving.

## Preparation

The first line a client sends to the server should be
a single newline-terminated string with the client's name.
This should be unique,
and is used by the server to allow reconnecting to an existing game.
You may send an empty string to connect as an "observer":
observers receive the same input as normal players,
but all communication from an observer to the server is ignored.

The server will not send any information until this string is sent.

## Server to Client

There are five kinds of messages that the server will send,
denoted by the `"type"` top-level field in the JSON packet.

### `"registration"`

Sent on initial handshake.
Provides any initial/immutable information about the game state.

Fields:

- `"world"`: The world map. An object with the following contents:
  - `"height"`: A positive integer with the width of the world map.
  - `"width"`: A positive integer with the width of the tile map.
  - `"map"`: An array of length `height * width` containing one string per tile.
    See [the definition in this file](src/game/world.rs) for available strings.
    The first element is the bottom-left corner of the world.
- `"player"`: A unique integer denoting the client's identifier.

Example:

```json
{
  "type": "registration",
  "player_id": 4,
  "world": {
    "height": 4,
    "width": 4,
    "map": ["Grass", "Grass", "Garden", "Neutral", /* ... 12 more */]
  }
}
```

### `"update"`

Sent regularly, providing an updated view of the current game state.
Returns all relevant mutable state information each time.

Most sub-objects have a `"position"` field,
which contains two integers `"x"` and `"y"`
denoting the location on the world map.

Fields:

- `"data"`: An object denoting the available data. Has the following fields:
  - `"bees"`: A list of living bees in the game. Each element is an object with:
    - `"id"`: A unique integer denoting the bee's identifier.
    - `"player"`: Who owns the bee.
    - `"energy"`: An integer for the remaining lifetime for the bee.
    - `"pollen"`: The amount of pollen the bee has collected so far.
    - `"position"`: The location of the bee.
  - `"hives"`: A list of spawners. Each element is an object with:
    - `"player"`: The owner of the spawner.
    - `"position"`: The location of the hive.
  - `"flowers"`: A list of flowers. Each element is an object with:
    - `"pollen"`: An integer, the amount of pollen that can still be collected.
    - `"is_pollinated"`: A boolean, whether this flower is pollinated or not.
    - `"position"`: The location of the flower.
  - `"birds"`: *TODO*.
  - `"cars"`: *TODO*.

Example:

```json
{
  "type": "update",
  "data": {
    "bees": [
      {
        "id": 71,
        "player": 4,
        "energy": 18,
        "pollen": 6,
        "position": {
          "x": 7,
          "y": 3
        }
      },
      // ...
    ],
    "hives": [
      {
        "player": 4,
        "position": {
          "x": 5,
          "y": 5
        }
      },
      // ...
    ],
    "flowers": [
      {
        "pollen": 3,
        "is_pollinated": false,
        "position": {
          "x": 7,
          "y": 3
        }
      },
      // ...
    ],
    "birds": [],
    "cars": [],
  }
}
```

### `"done"`

Notification that the game has finished successfully.
This will be sent just before stream closure.

This message has no other fields.

Example:

```json
{
  "type": "done"
}
```

### `"warning"`

An ignorable error has occurred.
You will still be connected to the game.

Fields:

- `"msg"`: description of the error.

Example:

```json
{
  "type": "warning",
  "msg": "Bad input"
}
```

### `"error"`

A fatal error has occurred.
This will be sent just before stream closure.

Fields:

- `"msg"`: description of the error.

Example:

```json
{
  "type": "error",
  "msg": "Game already finished"
}
```

## Client to Server

At this stage, the only message sent by the client are actions.
This is a list of movements for each bee controlled by the client.
All entries should be less than 8192 characters long;
longer transmissions will be rejected by the server.

Each message should be a list of objects,
where each object has the following two fields:

- `"bee"`: an integer identifying the bee to move.
- `"direction"`: what direction to move the bee.
  Should be one of `"North"`, `"South"`, `"East"`, or `"West"`,
  with the obvious meanings.
  May also be `null`, being an explicit "move nowhere".

Multiple updates inbetween state ticks overwrite each other;
for example, sending `[{"bee":1,"direction":"North"}]`
followed by `[{"bee":1,"direction":"South"}]`
will cause the denoted bee to move southwards for this game tick.
Any bees without an action provided for this tick
will not move anywhere;
this is equivalent to specifying `"direction": null` for the bee in question.

Example:

```json
[
  { "bee": 1, "direction": "North" },
  { "bee": 2, "direction": "West" },
  { "bee": 5, "direction": null },
  { "bee": 7 }  // same as specifying `"direction": null`.
]
```
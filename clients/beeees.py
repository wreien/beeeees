from __future__ import annotations

import asyncio
import json
import platform
from collections.abc import Iterable, Mapping
from enum import Enum, auto
from typing import Union, Iterator, Literal, Callable, NewType


BeeID = NewType("BeeID", int)
PlayerID = NewType("PlayerID", int)
Direction = Union[Literal["North", "East", "South", "West"], None]
Position = tuple[int, int]


def make_position(data) -> Position:
    """Create a new `Position` from the given dictionary."""
    return (data["x"], data["y"])


class Tile(Enum):
    """Kinds of tiles on the map."""

    Grass = auto()
    Garden = auto()
    Neutral = auto()
    Road = auto()
    Block = auto()
    SpawnPoint = auto()

    def __repr__(self) -> str:
        return f"<{self.__class__.__name__}.{self.name}>"

    def is_passable(self) -> bool:
        """Whether or not `Bee`s can pass over this tile."""
        return self != Tile.Block


class World(Mapping[Position, Tile]):
    """The world map, a mapping from `Position` to `Tile`.

    Attributes:
    - `width`: the width of the map.
    - `height`: the height of the map.
    - `[x, y]`: the `Tile` at position `(x, y)`.
    """

    def __init__(self, data):
        """Initialize the world from the given dictionary."""
        self.width = int(data["width"])
        self.height = int(data["height"])
        self._tiles = list(Tile[t] for t in data["map"])

    def __getitem__(self, key: Position) -> Tile:
        """Get the `Tile` at the given `Position`."""
        (x, y) = key
        if 0 <= x < self.width or 0 <= y < self.height:
            raise KeyError
        return self._tiles[x + y * self.width]

    def __iter__(self) -> Iterator[Position]:
        """Get an iterator over every valid position."""
        for row in range(self.width):
            for col in range(self.height):
                yield (row, col)

    def __len__(self) -> int:
        """How many tiles there are."""
        return self.width * self.height


class Bee(object):
    """A bee controlled by a player.

    Attributes:
    - `id`: Uniquely identifies the bee
    - `player`: Who owns this bee
    - `energy`: The remaining energy for the bee. Dies when reaches 0.
    - `pollen`: The amount of pollen held by this bee.
    - `position`: a `Position` with the bee's current location in the world.
    """

    def __init__(self, bee):
        """Initialize the bee from the given dictionary."""
        self.id = BeeID(bee["id"])
        self.player = PlayerID(bee["player"])
        self.energy = int(bee["energy"])
        self.pollen = int(bee["pollen"])
        self.position = make_position(bee["position"])


class Flower(object):
    """A flower that makes pollen.

    Attributes:
    - `pollen`: The amount of pollen that this flower can still make.
    - `is_pollinated`: Whether or not the flower's been pollinated yet.
    - `position`: a `Position` with the flower's location in the world.
    """

    def __init__(self, flower):
        """Initialize the flower from the given dictionary."""
        self.pollen = int(flower["pollen"])
        self.is_pollinated = bool(flower["is_pollinated"])
        self.position = make_position(flower["position"])


class Hive(object):
    """A player's "home".

    Attributes:
    - `player`: The player that this hive belongs to
    - `position`: a `Position` with the hive's location in the world.
    """

    def __init__(self, hive):
        """Initialize the hive from the given dictionary."""
        self.player = PlayerID(hive["player"])
        self.position = make_position(hive["position"])


class Entities(object):
    """All entities currently active in the game.

    Attributes:
    - `bees`: A list of living `Bee`s.
    - `flowers`: A list of living `Flower`s.
    - `hives`: A list of player `Hive`s.
    """

    bees: list[Bee]
    flowers: list[Flower]
    hives: list[Hive]

    def __init__(self, data):
        """Initialize the entity collection from the given dictionary."""
        self.bees = [Bee(b) for b in data["bees"]]
        self.flowers = [Flower(f) for f in data["flowers"]]
        self.hives = [Hive(h) for h in data["hives"]]

    def bees_for(self, player: PlayerID) -> Iterable[Bee]:
        """Get an iterable of bees, filtered for just the given player."""
        for bee in self.bees:
            if bee.player == player:
                yield bee

    def hive_for(self, player: PlayerID) -> Hive:
        """Get the hive (spawn point) for the given player."""
        return next(h for h in self.hives if h.player == player)


class Error(Exception):
    """An error within the `beeees` module.

    Attributes:
    - `message`: human-readable description of the error.
    """

    def __init__(self, message: str):
        """Creates the error with given message."""
        self.message = message


class ConnectionError(Error):
    """Signals that the connection dropped."""

    def __init__(self):
        super().__init__("connection dropped")


class Connection(object):
    """Wraps a connection with the server.

    Attributes:
    - `reader`: The read half of the connection.
    - `writer`: The write half of the connection.
    """

    reader: asyncio.StreamReader
    writer: asyncio.StreamWriter

    @classmethod
    async def create(cls, host: str, port: Union[int, str]) -> Connection:
        """Open a new connection with the given `host` and `port`."""
        self = cls()
        self.reader, self.writer = await asyncio.open_connection(host, port)
        return self

    async def write(self, msg: Union[str, bytes]) -> None:
        """Write a message to the connection.

        If passed a string, will encode using UTF-8.
        All messages are terminated with a newline.
        Will flush the stream before finishing.
        """
        if isinstance(msg, bytes):
            self.writer.write(msg)
        else:
            self.writer.write(msg.encode("utf-8"))
        self.writer.write(b"\n")
        await self.writer.drain()

    async def write_json(self, data) -> None:
        """Write a blob as JSON-encoded data to the connection.

        Uses the same encoding and flushing behaviour as `write`.
        """
        print(f"Sending: {data}")
        await self.write(json.dumps(data, separators=(",", ":")))

    async def read(self) -> str:
        """Read a message from the connection.

        Returns a single newline-terminated UTF-8 encoded string.
        If the connection has been dropped, raises `ConnectionError`.
        """
        line = await self.reader.readline()
        if not line:
            raise ConnectionError()
        return line.decode("utf-8")


Moves = dict[BeeID, Direction]
StepFunc = Callable[[PlayerID, World, Entities], Moves]


class Client(object):
    """Encapsulates a client program in the game.

    Attributes:
    - `conn`: The `Connection` to the server.
    - `id`: The client's player ID.
    - `world`: The (immutable) game world.
    """

    conn: Connection
    id: PlayerID
    world: World

    @classmethod
    async def register(cls, conn: Connection, name: str) -> Client:
        """Create a new client.

        Registers to the server over the provided connection `conn`.
        Specifies the player name as `name`;
        this can be used to reconnect to an existing session later on.
        """
        self = cls()
        self.conn = conn

        await self.conn.write_json({"type": "register", "name": name})
        msg = await self.conn.read()
        packet = json.loads(msg)
        if packet["type"] == "done":
            raise Error("game already finished")
        elif packet["type"] == "error":
            raise Error(packet["msg"])
        elif packet["type"] == "registration":
            self.id = PlayerID(packet["player"])
            self.world = World(packet["world"])
        else:
            raise Error(f"unexpected message on registration: {msg}")

        return self

    async def run(self, step: StepFunc) -> None:
        """Run the game.

        Handles communication with the server
        and responds to messages appropriately.
        Uses the provided `step` function to control bees.
        """
        while True:
            msg = await self.conn.read()
            packet = json.loads(msg)
            if packet["type"] == "done":
                print("Received finish signal")
                return
            elif packet["type"] == "warning":
                print("Received warning:", packet["msg"])
            elif packet["type"] == "error":
                print(f"Recevied error:", packet["msg"])
                return
            elif packet["type"] == "update":
                entities = Entities(packet["data"])
                moves = step(self.id, self.world, entities)
                data = {
                    "type": "moves",
                    "moves": list(
                        {"bee": k, "direction": v} for (k, v) in moves.items()
                    ),
                }
                await self.conn.write_json(data)
            else:
                raise Error(f"unknown message: {msg}")


def play(name: str, host: str, port: Union[int, str], step: StepFunc) -> None:
    """Play the game as a new (or returning) client.

    Connects to the server with given `host` and `port`.
    Registers as the player identified by `name`:
    This is used to reconnect to an existing session if dropping out early.

    The bulk of the work is handled by the provided `step` function.
    Each "round" the function will be provided with
    their player ID, the world map, and the current positions of all entities;
    the function should return a list of movements to be made
    by all bees owned by their player ID.

    The movements are specified by the four cardinal directions:
    "North", "South", "East", or "West".
    The return should be a dictionary from a given `BeeID` to a direction.
    For example, a step function that always moves south might look like:

    ```python
    def always_south(player, world, entities):
        moves = {}
        for bee in entities.bees:
            if bee.player == player:
                moves[bee.id] = "South"
        return moves
    ```
    """

    async def main() -> None:
        try:
            conn = await Connection.create(host, port)
            client = await Client.register(conn, name)
            await client.run(step)
        except Error as e:
            print(f"Fatal error: {e.message}")

    try:
        if platform.system() == "Windows":
            # just don't handle Proactor always throwing exception on teardown
            asyncio.get_event_loop().run_until_complete(main())
        else:
            asyncio.run(main())
    except KeyboardInterrupt:
        print("Interrupted.")

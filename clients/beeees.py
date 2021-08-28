from __future__ import annotations

import asyncio
import json
import platform
from collections.abc import Mapping
from typing import Union, Iterator, Literal, Callable, NewType


BeeID = NewType("BeeID", int)
PlayerID = NewType("PlayerID", int)
Direction = Literal["North", "East", "South", "West"]
Position = tuple[int, int]


def make_position(data) -> Position:
    return (data["x"], data["y"])


class World(Mapping[Position, str]):
    def __init__(self, data):
        self.width = int(data["width"])
        self.height = int(data["height"])
        self.tiles = list[str](data["map"])

    def __getitem__(self, key: Position) -> str:
        (x, y) = key
        if 0 <= x < self.width or 0 <= y < self.height:
            raise KeyError
        return self.tiles[x + y * self.width]

    def __iter__(self) -> Iterator[Position]:
        for row in range(self.width):
            for col in range(self.height):
                yield (row, col)

    def __len__(self) -> int:
        return self.width * self.height


class Bee(object):
    def __init__(self, bee):
        self.id = BeeID(bee["id"])
        self.player = PlayerID(bee["player"])
        self.energy = int(bee["energy"])
        self.pollen = int(bee["pollen"])
        self.position = make_position(bee["position"])


class Flower(object):
    def __init__(self, flower):
        self.pollen = int(flower["pollen"])
        self.is_pollinated = bool(flower["is_pollinated"])
        self.position = make_position(flower["position"])


class Hive(object):
    def __init__(self, hive):
        self.player = PlayerID(hive["player"])
        self.position = make_position(hive["position"])


class Entities(object):
    bees: list[Bee]
    flowers: list[Flower]
    hives: list[Hive]

    def __init__(self, data):
        self.bees = [Bee(b) for b in data["bees"]]
        self.flowers = [Flower(f) for f in data["flowers"]]
        self.hives = [Hive(h) for h in data["hives"]]


class Error(Exception):
    def __init__(self, message: str):
        self.message = message


class ConnectionError(Error):
    def __init__(self):
        super().__init__("connection dropped")


class Connection(object):
    reader: asyncio.StreamReader
    writer: asyncio.StreamWriter

    @classmethod
    async def create(cls, host: str, port: Union[int, str]) -> Connection:
        self = cls()
        self.reader, self.writer = await asyncio.open_connection(host, port)
        return self

    async def write(self, msg: Union[str, bytes]) -> None:
        if isinstance(msg, bytes):
            self.writer.write(msg)
        else:
            self.writer.write(msg.encode("utf-8"))
        self.writer.write(b"\n")
        await self.writer.drain()

    async def read(self) -> str:
        line = await self.reader.readline()
        if not line:
            raise ConnectionError()
        return line.decode("utf-8")


Moves = dict[BeeID, Direction]
StepFunc = Callable[[PlayerID, World, Entities], Moves]


class Client(object):
    conn: Connection
    id: PlayerID
    world: World

    @classmethod
    async def register(cls, conn: Connection, name: str) -> Client:
        self = cls()
        self.conn = conn

        await self.conn.write(name)
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
                l = list({"bee": k, "direction": v} for (k, v) in moves.items())
                print(f"Sending: {l}")
                await self.conn.write(json.dumps(l, separators=(",", ":")))
            else:
                raise Error(f"unknown message: {msg}")


def play(name: str, host: str, port: Union[int, str], step: StepFunc) -> None:
    async def main() -> None:
        try:
            conn = await Connection.create(host, port)
            client = await Client.register(conn, name)
            await client.run(step)
        except Error as e:
            print(f"Fatal error: {e.message}")

    if platform.system() == "Windows":
        # just don't handle Proactor always throwing exception on teardown
        asyncio.get_event_loop().run_until_complete(main())
    else:
        asyncio.run(main())

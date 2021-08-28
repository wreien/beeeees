#!/usr/bin/env python3

from beeees import World, Entities, PlayerID, Moves
import random

PLAYER_NAME = "Jim"
SERVER_HOST = "127.0.0.1"
SERVER_PORT = 49998


def move_randomly(player: PlayerID, world: World, entities: Entities) -> Moves:
    result: Moves = {}
    for bee in entities.bees:
        if bee.player == player:
            result[bee.id] = random.choice(["North", "South", "East", "West"])
    return result


if __name__ == "__main__":
    from beeees import play

    play(PLAYER_NAME, SERVER_HOST, SERVER_PORT, move_randomly)

#!/usr/bin/env python3

from beeees import World, Entities, Bee, PlayerID, Moves, Position, Direction

PLAYER_NAME = "Bob"
SERVER_HOST = "127.0.0.1"
SERVER_PORT = 49998


def step_to_point(start: Position, end: Position) -> tuple[int, Direction]:
    """Get the direction to go to arrive at the given end point.

    Returns a tuple `(distance, direction)`, where
    - `distance` is the number of steps required (assuming nothing changes), and
    - `direction` is the direction of the first step to take
    """
    xdist = abs(end[0] - start[0])
    ydist = abs(end[1] - start[1])
    total = xdist + ydist
    if start[0] > end[0]:
        return (total, "West")
    elif start[0] < end[0]:
        return (total, "East")
    elif start[1] > end[1]:
        return (total, "South")
    elif start[1] < end[1]:
        return (total, "North")
    else:
        # we must already be there
        return (total, None)


def step_to_nearest_flower(bee: Bee, entities: Entities) -> Direction:
    """Return the direction for the given bee to move towards the nearest flower."""
    choices = (
        step_to_point(bee.position, flower.position) for flower in entities.flowers
    )
    return min(choices, key=lambda x: x[0], default=(0, None))[1]


def move_to_nearest(player: PlayerID, world: World, entities: Entities) -> Moves:
    """Move all bees to nearest flower. If the bee has pollen, instead go home."""
    result: Moves = {}
    my_hive = entities.hive_for(player)
    for bee in entities.bees_for(player):
        if bee.pollen > 0 or len(entities.flowers) == 0:
            result[bee.id] = step_to_point(bee.position, my_hive.position)[1]
        else:
            result[bee.id] = step_to_nearest_flower(bee, entities)
    return result


if __name__ == "__main__":
    from beeees import play

    play(PLAYER_NAME, SERVER_HOST, SERVER_PORT, move_to_nearest)

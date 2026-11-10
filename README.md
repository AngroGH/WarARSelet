# WarARSelet

WarARSelet is a Rust reimplementation of *War Selection*'s RTS formula, with a
focus on the things the Russian competitive community needs: deterministic
simulation, replays, server authority and headless tooling.

## Status

The project is playable **headlessly** right now:

- `warsel-core` — deterministic authoritative simulation (economy, construction,
  production, combat, epochs, civilizations, fog, maps, replays, checksums).
- `warsel-cli` — bot demo matches, simulations, ASCII map previews, benchmarks.
- `warsel-server` — authoritative 20 Hz TCP server with a JSON-line protocol.

There is no graphical client or lobby yet. The server is a development tool, not
a hardened production service.

## Quick start

```bash
# Watch a 180-second bot match in the terminal.
cargo run -p warsel-cli

# A longer match with two bots.
cargo run -p warsel-cli -- demo --seconds 300 --players 2

# Headless simulation with a JSON report and a saved replay.
cargo run -p warsel-cli -- simulate --seed 7 --players 4 --seconds 120 --json --replay match.json

# ASCII map preview.
cargo run -p warsel-cli -- map --seed 7 --players 2

# Simulation throughput benchmark.
cargo run -p warsel-cli -- bench --players 8 --ticks 1200

# Start the authoritative dev server.
cargo run -p warsel-server -- --players 2
```

### CLI options

```
warsel [demo|simulate|map|bench] [OPTIONS]

--seed <N>        Map/game seed            (default: 12345)
--players <N>     Player count, 1..=62     (default: 2)
--seconds <N>     Match length in seconds  (default: 180)
--ticks <N>       Bench tick count         (overrides --seconds)
--mode <MODE>     ffa | team | armageddon | survival | sandbox
--size <SIZE>     duel | small | standard | large
--replay <PATH>   Write the command replay as JSON
--json            Print the final report as JSON
--quiet           Suppress the live demo output
```

`demo` and `simulate` run deterministic bots. The bots issue ordinary commands;
the world remains authoritative and the command stream is verified against
checkpoints at the end of the match.

## Server protocol

`warsel-server` listens on `127.0.0.1:7777` by default and advances the match at
20 ticks per second. Messages are newline-delimited JSON over TCP.

Each connection is assigned a player slot (`welcome` contains `player`). Extra
connections become spectators. Bots play every slot that has no human attached,
and resume when a human disconnects.

Client messages:

```json
{"type":"ping"}
{"type":"chat","text":"hello"}
{"type":"command","command":{"Chat":{"text":"hello"}}}
```

Server messages:

```json
{"type":"welcome","protocol":1,"player":1}
{"type":"state","tick":20,"outcome":"Ongoing","players":[...]}
{"type":"events","tick":21,"events":[...],"dropped":0}
{"type":"result","player":1,"accepted":true,"reason":null}
{"type":"over","tick":4200,"outcome":{"Victory":1}}
```

`command` is the serde representation of `warsel_core::Command`. For example, a
move command targets entity ids:

```json
{"type":"command","command":{"Move":{"units":[{"index":12,"generation":0}],"target":{"x":80,"y":48}}}}
```

Server options:

```
--addr <ADDR>     Listen address          (default: 127.0.0.1:7777)
--players <N>     Player slots, 1..=62    (default: 2)
--seed <N>        Match seed              (default: 12345)
--mode <MODE>     ffa | team | armageddon | survival | sandbox
--size <SIZE>     duel | small | standard | large
--no-bots         Wait for human players instead of running bots
```

## Architecture

```
client ──commands──► World::apply_command() ──► World::step() ──► events ──► client
                              │
                     map / entities / players
```

Rules that keep the core trustworthy:

- **No floating point in gameplay.** Positions and times are integer fixed-point
  (`Fixed`, Q24.8). Floating point is only used for presentation.
- **Server authority.** Clients send commands; the world validates ownership,
  epoch, resources, population, bounds and queue limits.
- **Deterministic.** Seeded SplitMix64 RNG, ordered iteration, FNV-1a state
  hashes, replays as `config + seed + command stream`.
- **Bounded work per tick.** Spatial hash grid, budgeted A*, capped event
  buffer, fixed-size production queues.
- **Generational entity ids.** A stale id can never resolve to a recycled slot.

Core modules: `arena`, `bot`, `checksum`, `command`, `defs`, `entity`, `epoch`,
`fog`, `map`, `math`, `pathfinding`, `player`, `replay`, `resource`, `rng`,
`spatial`, `time`, `world`.

## Tests

```bash
cargo test --workspace
```

The suite covers map generation, pathfinding, the damage triangle, economy,
construction, production queues, epoch gates, civilization choices, fog,
elimination, Armageddon, determinism, replay verification, snapshots and a
small bot smoke test.

## Known gaps

- No graphical client, lobby, matchmaking or account system.
- Wonder victory timer and Survival wave ownership are not finished.
- Fog is enforced for client visibility, but the simulation itself is
  omniscient when acquiring targets.
- The dev server has no authentication or rate limiting.

## License

MIT. See `LICENSE`.

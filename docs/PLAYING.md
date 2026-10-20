# Running the game

Start the native client from the repository root:

```sh
cargo run
```

Choose **Play solo** to test movement, or start a relay for multiplayer:

```sh
cargo run -p rts_server --bin rts-server -- 127.0.0.1:9001
```

The optional bind address defaults to `127.0.0.1:9001`; `RTS_SERVER_ADDR` supplies a default when no argument is passed. Bind to `0.0.0.0:9001` for LAN access. On each client enter `ws://<relay-address>:9001`. The host creates a room and shares its six-character code; the other player joins. The host starts when both slots are filled. Clients must have identical map and rules content.

On Android 17 and newer, allow Nearby devices access when the app requests it to connect to LAN relays. If denied, solo play and public Internet relays remain available; grant the permission in system app settings before retrying a LAN relay.

For a browser client, install Trunk and the wasm target, then run:

```sh
rustup target add wasm32-unknown-unknown
trunk serve
```

Open the address Trunk prints. A browser served over HTTPS needs a `wss://` relay behind a TLS reverse proxy. Native clients currently support `ws://` only.

When hosting the built web files elsewhere, the server must support HTTP byte-range requests (`206 Partial Content`) for `assets.pack`. The asset pack loader reads its index and individual assets on demand; a basic server that always returns the whole file is insufficient.

Select units by click/tap or drag, then right-click ground on desktop or tap ground on touch to move. Two fingers pan and pinch zoom. Stop and Clear are in the bottom bar; Save replay and Leave are in the top bar. Touching Server address or Room code opens a keypad with Done and Cancel. The lobby and keypad can scroll on shorter screens.

Each tile holds four size points: a worker uses one and a soldier uses two. Four workers, two soldiers, or two workers with one soldier can share a tile. Units route around full tiles and wait if a route or destination stays full; they retry automatically as space opens.

Terrain movement is configured with `speed_modifier` in `assets/data/core.rules.ron`: `1.0` is normal speed, `0.0` blocks movement, `2.0` doubles speed, and `0.5` halves it. Units choose routes by travel time, so a longer fast route can beat a short slow one.

**Replay last** plays the latest in-memory recording. **Save replay** exports the turns played so far. Native **Load saved replay** reads the saved file; browser **Load replay** opens a file picker. Playback verifies the final state hash and accepts no unit orders.

Native files are stored at:

- Linux: `$XDG_DATA_HOME/rts/replays/latest.replay.ron`, falling back to `~/.local/share/rts/replays/latest.replay.ron`.
- Windows: `%LOCALAPPDATA%/rts/replays/latest.replay.ron` (unless `XDG_DATA_HOME` is set).
- Android: `replays/latest.replay.ron` inside the app's internal data directory.
- Other native platforms: `$XDG_DATA_HOME`, then `LOCALAPPDATA`, then `~/.local/share`, with the same `rts/replays/latest.replay.ron` suffix.

Browser saves download `match.replay.ron`. Leaving a match preserves its recording until another match is recorded. Replays contain the original map and rules; asset textures remain local presentation data.

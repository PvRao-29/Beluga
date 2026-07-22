# beluga-wasm

WebAssembly bindings for Beluga. Single-threaded search (no Lazy SMP).

## Build

Requires [rustup](https://rustup.rs/) with the `wasm32-unknown-unknown` target and [`wasm-pack`](https://rustwasm.github.io/wasm-pack/):

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack

cd engine-wasm
wasm-pack build --target web --release
```

Output lands in `pkg/` (`beluga_wasm.js` + `beluga_wasm_bg.wasm`, ~117KB).

## JS usage

```js
import init, { BelugaEngine } from "./pkg/beluga_wasm.js";

await init();
const engine = new BelugaEngine(16); // hash size in MB (1–64)

engine.new_game();
engine.make_move("e2e4");
const reply = engine.go_depth(12); // UCI best move, e.g. "e7e5"
engine.make_move(reply);

console.log(engine.fen(), engine.result());
```

### API

| Method | Notes |
| --- | --- |
| `new(hash_mb)` | Construct; inits magic bitboards |
| `new_game()` | Startpos + clear TT / heuristics |
| `set_fen(fen)` | Load a position |
| `fen()` / `side_to_move()` | `"w"` or `"b"` |
| `legal_moves()` | Space-separated UCI moves |
| `make_move(uci)` | `false` if illegal |
| `go_depth(n)` | Preferred for browser latency |
| `go_movetime(ms)` | Wall-clock search |
| `is_check()` / `is_game_over()` / `result()` | `result` → `checkmate` \| `stalemate` \| `draw` \| `""` |

Prefer `go_depth` over `go_movetime` when embedding in a UI — run search off the main thread (Web Worker) so the page stays responsive.

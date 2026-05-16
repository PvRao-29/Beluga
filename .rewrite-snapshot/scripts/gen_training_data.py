#!/usr/bin/env python3
"""Generate (fen, search_eval, game_result) training records via Beluga self-play.

This is the data-generation hook for both Texel tuning of the handcrafted eval and
for training an NNUE network against Beluga's feature layout. It drives the engine
over UCI, plays fast self-play games from random openings, records the side-to-move
eval at each quiet position, and labels every position with the final game result.

Output: JSONL lines of {"fen": str, "eval_cp": int, "result": float}
where result is from White's perspective (1.0 win, 0.5 draw, 0.0 loss).

Requires: python-chess  (pip install chess)

Usage:
    python scripts/gen_training_data.py --engine target/release/beluga \\
        --games 100 --depth 8 --out data.jsonl
"""
import argparse
import json
import random
import subprocess
import sys

try:
    import chess
except ImportError:
    sys.exit("This script needs python-chess: pip install chess")


class UciEngine:
    def __init__(self, path):
        self.p = subprocess.Popen(
            [path], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            text=True, bufsize=1,
        )
        self._send("uci")
        self._wait("uciok")
        self._send("isready")
        self._wait("readyok")

    def _send(self, cmd):
        self.p.stdin.write(cmd + "\n")
        self.p.stdin.flush()

    def _wait(self, token):
        for line in self.p.stdout:
            if line.strip().startswith(token):
                return line.strip()
        raise EOFError(f"engine closed before '{token}'")

    def bestmove(self, fen, depth):
        self._send("position fen " + fen)
        self._send(f"go depth {depth}")
        score_cp = 0
        best = None
        for line in self.p.stdout:
            line = line.strip()
            if line.startswith("info") and " score cp " in line:
                toks = line.split()
                score_cp = int(toks[toks.index("cp") + 1])
            elif line.startswith("bestmove"):
                best = line.split()[1]
                break
        return best, score_cp

    def quit(self):
        self._send("quit")
        self.p.wait(timeout=5)


def random_opening(plies):
    board = chess.Board()
    for _ in range(plies):
        moves = list(board.legal_moves)
        if not board.is_game_over() and moves:
            board.push(random.choice(moves))
        else:
            break
    return board


def play_game(engine, depth, opening_plies):
    board = random_opening(opening_plies)
    records = []  # (fen, eval_cp_white_pov)
    while not board.is_game_over(claim_draw=True) and board.fullmove_number < 200:
        best, score_cp = engine.bestmove(board.fen(), depth)
        if best is None or best == "0000":
            break
        # Convert stm-relative eval to White's perspective.
        white_cp = score_cp if board.turn == chess.WHITE else -score_cp
        if not board.is_check():
            records.append((board.fen(), white_cp))
        try:
            board.push_uci(best)
        except ValueError:
            break

    result = board.result(claim_draw=True)
    score = {"1-0": 1.0, "0-1": 0.0, "1/2-1/2": 0.5}.get(result, 0.5)
    return [(fen, cp, score) for fen, cp in records]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", required=True)
    ap.add_argument("--games", type=int, default=100)
    ap.add_argument("--depth", type=int, default=8)
    ap.add_argument("--opening-plies", type=int, default=8)
    ap.add_argument("--out", default="data.jsonl")
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()
    random.seed(args.seed)

    engine = UciEngine(args.engine)
    n = 0
    with open(args.out, "w") as f:
        for g in range(args.games):
            for fen, cp, result in play_game(engine, args.depth, args.opening_plies):
                f.write(json.dumps({"fen": fen, "eval_cp": cp, "result": result}) + "\n")
                n += 1
            print(f"game {g + 1}/{args.games}  positions={n}", file=sys.stderr)
    engine.quit()
    print(f"wrote {n} records to {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Register or update the Beluga engine in En Croissant's engines.json."""

from __future__ import annotations

import json
import os
import platform
import sys
from pathlib import Path


ENGINE_ID = "beluga-dev"
DEFAULT_GO = {"t": "Depth", "c": 14}
DEFAULT_SETTINGS = [
    {"name": "MultiPV", "value": "1"},
    {"name": "Threads", "value": str(os.cpu_count() or 1)},
    {"name": "Hash", "value": "128"},
]


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def beluga_binary(root: Path) -> Path:
    release = root / "target" / "release" / "beluga"
    if release.exists():
        return release
    debug = root / "target" / "debug" / "beluga"
    if debug.exists():
        print(
            f"warning: using debug build at {debug} (release binary not found)",
            file=sys.stderr,
        )
        return debug
    raise FileNotFoundError(
        "Beluga binary not found at target/release/beluga or target/debug/beluga\n"
        "Build it first: cargo build --release -p beluga-uci"
    )


def beluga_version(root: Path) -> str:
    cargo = root / "Cargo.toml"
    for line in cargo.read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("version = "):
            return line.split("=", 1)[1].strip().strip('"')
    return "0.0.0"


def engines_json_path() -> Path:
    system = platform.system()
    if system == "Darwin":
        base = Path.home() / "Library" / "Application Support" / "org.encroissant.app"
    elif system == "Linux":
        xdg = os.environ.get("XDG_DATA_HOME")
        base = Path(xdg) if xdg else Path.home() / ".local" / "share"
        base = base / "org.encroissant.app"
    elif system == "Windows":
        appdata = os.environ.get("APPDATA")
        if not appdata:
            raise RuntimeError("APPDATA is not set")
        base = Path(appdata) / "org.encroissant.app"
    else:
        raise RuntimeError(f"unsupported platform: {system}")

    return base / "engines" / "engines.json"


def load_engines(path: Path) -> list[dict]:
    if not path.exists():
        return []
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise RuntimeError(f"expected array in {path}")
    return data


def beluga_entry(root: Path) -> dict:
    binary = beluga_binary(root).resolve()
    if not binary.exists():
        raise FileNotFoundError(f"Beluga binary not found at {binary}")

    return {
        "type": "local",
        "id": ENGINE_ID,
        "name": "Beluga",
        "version": beluga_version(root),
        "path": str(binary),
        "loaded": True,
        "enabled": True,
        "go": DEFAULT_GO,
        "settings": DEFAULT_SETTINGS,
    }


def merge_engines(engines: list[dict], entry: dict) -> list[dict]:
    out = [e for e in engines if e.get("id") != ENGINE_ID]
    out.insert(0, entry)
    return out


def main() -> int:
    root = repo_root()
    path = engines_json_path()
    path.parent.mkdir(parents=True, exist_ok=True)

    entry = beluga_entry(root)
    engines = merge_engines(load_engines(path), entry)
    path.write_text(json.dumps(engines, indent=2) + "\n", encoding="utf-8")

    print(f"Registered Beluga at {entry['path']}")
    print(f"En Croissant engines file: {path}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (FileNotFoundError, RuntimeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc

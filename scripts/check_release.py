"""Offline release guards. No credentials or runtime files are read."""
import os
from pathlib import Path
import subprocess
import tomllib

tracked = subprocess.check_output(["git", "ls-files", "-z"]).decode().split("\0")
for name in tracked:
    assert not name.startswith(("maps/", "school_roads/")), f"Runtime data tracked: {name}"
    assert name not in {"session.json", "identity.json", "config.json", "points_cache.json"}, f"Runtime data tracked: {name}"
    assert not (name.startswith("diagnostic-") and name.endswith(".json") and "/" not in name), f"Runtime data tracked: {name}"
tag = os.environ.get("GITHUB_REF", "")
if tag.startswith("refs/tags/v"):
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    assert tag.removeprefix("refs/tags/v") == version, "Release tag must match Cargo package version"
print("Release guards passed")

"""Package only an explicit executable and public first-run files."""
import hashlib
from pathlib import Path
import sys
import zipfile


def package(executable, destination):
    root = Path(__file__).resolve().parent.parent
    files = {
        "NekoSportsWorldTool.exe": Path(executable).read_bytes(),
        "config.example.json": (root / "config.example.json").read_bytes(),
        "更新说明.txt": (root / "docs/windows-first-run.txt").read_bytes(),
    }
    assert files["NekoSportsWorldTool.exe"][:2] == b"MZ", "Expected Windows executable"
    sums = "".join(f"{hashlib.sha256(data).hexdigest()}  {name}\n" for name, data in files.items())
    files["SHA256SUMS.txt"] = sums.encode()
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in files.items():
            archive.writestr(name, data)
    destination.with_suffix(".sha256").write_text(f"{hashlib.sha256(destination.read_bytes()).hexdigest()}  {destination.name}\n")
    return files


if __name__ == "__main__":
    package(*sys.argv[1:])

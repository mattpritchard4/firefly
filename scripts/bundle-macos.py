#!/usr/bin/env python3
"""Bundle an already built example for native macOS inspection; never compiles."""
import argparse
import hashlib
from pathlib import Path
import plistlib
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("example", choices=sorted(p.stem for p in Path("examples").glob("*.rs")))
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
binary = root / "target/debug/examples" / args.example
bundle = root / "target" / f"Firefly {args.example}.app"
macos = bundle / "Contents/MacOS"
macos.mkdir(parents=True, exist_ok=True)
shutil.copy2(binary, macos / args.example)
shutil.copytree(root / "assets", macos / "assets", dirs_exist_ok=True)
with (bundle / "Contents/Info.plist").open("wb") as file:
    plistlib.dump({
        "CFBundleExecutable": args.example,
        "CFBundleIdentifier": f"com.blg.firefly.{args.example}",
        "CFBundleName": f"Firefly {args.example}",
        "CFBundlePackageType": "APPL",
        "NSHighResolutionCapable": True,
    }, file)
print(bundle)
print("sha256", hashlib.sha256(binary.read_bytes()).hexdigest())

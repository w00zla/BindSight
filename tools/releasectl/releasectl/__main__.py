"""Entry point: python -m releasectl [--dry-run] [--root <repo>]."""

import argparse
import subprocess
from pathlib import Path

from .app import ReleaseCtl


def repo_root() -> Path:
    out = subprocess.run(["git", "rev-parse", "--show-toplevel"],
                         capture_output=True, text=True)
    if out.returncode == 0:
        return Path(out.stdout.strip())
    return Path(__file__).resolve().parents[3]


def main() -> None:
    parser = argparse.ArgumentParser(prog="releasectl",
                                     description="BindSight channels and releases")
    parser.add_argument("--dry-run", action="store_true", help="start in dry-run mode")
    parser.add_argument("--root", type=Path, help="repository root (default: git toplevel)")
    args = parser.parse_args()
    ReleaseCtl(args.root or repo_root(), dry_run=args.dry_run).run()


if __name__ == "__main__":
    main()

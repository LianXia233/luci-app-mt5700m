#!/usr/bin/env python3
"""Re-emit `cargo test` diagnostics as GitHub annotations.

CI logs live behind a signed blob-storage URL that some networks (and some
maintainers' tooling) cannot fetch. GitHub annotations, in contrast, are served
by the API as small JSON objects, so a failing Rust build stays debuggable:
every `error[E…]` / `error:` / failing-test header is echoed as an `::error::`
with its **whole** diagnostic block (rustc's `expected/found` detail lives on
the caret line, several lines down — a 4-line window silently drops the one
line that says what is wrong).

`warning: unused …` blocks are emitted as `::warning::` with their location, so
dead imports from a mechanical refactor are visible without a second run.

Usage: ci-annotate-cargo.py <cargo-log-file> [max-annotations]
"""
import re
import sys
from pathlib import Path

ERROR = re.compile(
    r"^(error(\[[A-Z0-9]+\]|:)|test .* FAILED|---- .* stdout ----|thread '.*' panicked)"
)
WARNING = re.compile(r"^warning: unused")

# rustc separates diagnostics with a blank line; the caret line that carries
# `expected X, found Y` is up to ~8 lines into the block.
MAX_BLOCK_LINES = 12


def block(lines, start):
    """Diagnostic block starting at `lines[start]`, up to the first blank line."""
    out = []
    for line in lines[start:start + MAX_BLOCK_LINES]:
        if out and not line.strip():
            break
        out.append(line.strip())
    return " | ".join(l for l in out if l)


def main() -> int:
    log = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/cargo.log")
    limit = int(sys.argv[2]) if len(sys.argv) > 2 else 25
    lines = log.read_text(errors="replace").splitlines()

    shown = 0
    for i, line in enumerate(lines):
        stripped = line.strip()
        if ERROR.match(stripped):
            message = block(lines, i).replace("%", "%25").replace("\r", "")[:900]
            print(f"::error::{message}")
        elif WARNING.match(stripped):
            message = block(lines, i).replace("%", "%25").replace("\r", "")[:600]
            print(f"::warning::{message}")
        else:
            continue
        shown += 1
        if shown >= limit:
            break

    if shown == 0:
        tail = " | ".join(l.strip() for l in lines[-8:] if l.strip())
        print("::error::cargo test failed: " + tail[:900].replace("%", "%25"))
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Re-emit `cargo test` diagnostics as GitHub annotations.

CI logs live behind a signed blob-storage URL that some networks (and some
maintainers' tooling) cannot fetch. GitHub annotations, in contrast, are served
by the API as small JSON objects, so a failing Rust build stays debuggable:
every `error[E…]` / `error:` line is echoed as `::error::` with its source
context, which also shows up in the run summary.

Usage: ci-annotate-cargo.py <cargo-log-file> [max-annotations]
"""
import re
import sys
from pathlib import Path

DIAGNOSTIC = re.compile(
    r"^(error(\[[A-Z0-9]+\]|:)|warning: unused|test .* FAILED|---- .* stdout ----|"
    r"thread '.*' panicked)"
)
CONTEXT_LINES = 4


def main() -> int:
    log = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/cargo.log")
    limit = int(sys.argv[2]) if len(sys.argv) > 2 else 25
    lines = log.read_text(errors="replace").splitlines()

    shown = 0
    for i, line in enumerate(lines):
        if not DIAGNOSTIC.match(line.strip()):
            continue
        context = " | ".join(l.strip() for l in lines[i:i + CONTEXT_LINES] if l.strip())
        message = context.replace("%", "%25").replace("\r", "")[:900]
        print(f"::error::{message}")
        shown += 1
        if shown >= limit:
            break

    if shown == 0:
        tail = " | ".join(l.strip() for l in lines[-8:] if l.strip())
        print("::error::cargo test failed: " + tail[:900].replace("%", "%25"))
    return 0


if __name__ == "__main__":
    sys.exit(main())

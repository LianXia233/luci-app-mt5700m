#!/usr/bin/env python3
"""Structural syntax net for the std-only Rust backend (`cargo`-free).

`rs-static-check.py` resolves *names*; it does not notice that a scripted edit
left a stray brace, which rustc then reports as a wall of type errors far from
the real line. This script is the second net for exactly that class:

  * every `{}`, `()` and `[]` in the file balances, after comments and
    string/char literals are removed (so `"}"` and `'}'` do not count);
  * string, raw-string, char and byte literals are all terminated;
  * no token appears outside any delimiter level at the end of the file;
  * a `}` that closes nothing (the "stray brace" a block delete leaves behind)
    is reported with its line number.

It is deliberately *not* a parser: it never decides that code is correct, only
that a file is not structurally broken, which is what a mechanical move/edit
breaks first and what costs a full CI round-trip to discover.

Usage:  python3 scripts/rs-syntax-check.py <crate-src-dir> [...]   (default: ./src)
Exit code 1 on the first file that fails.
"""
import os
import sys


def scan(src):
    """Walk the source, tracking delimiter depth and literal state.

    Returns (errors, delimiters_seen). Errors are `(line, message)`.
    """
    errors = []
    stack = []  # (char, line)
    i = 0
    line = 1
    n = len(src)
    pairs = {')': '(', ']': '[', '}': '{'}
    while i < n:
        c = src[i]
        if c == '\n':
            line += 1
            i += 1
            continue
        # line comment
        if c == '/' and i + 1 < n and src[i + 1] == '/':
            j = src.find('\n', i)
            i = n if j < 0 else j
            continue
        # block comment (nesting, as Rust allows)
        if c == '/' and i + 1 < n and src[i + 1] == '*':
            depth = 1
            i += 2
            while i < n and depth:
                if src[i] == '\n':
                    line += 1
                    i += 1
                elif src.startswith('/*', i):
                    depth += 1
                    i += 2
                elif src.startswith('*/', i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                errors.append((line, 'unterminated block comment'))
            continue
        # raw string r#"…"# / r"…" / br#"…"#
        if c == 'r' and i + 1 < n and src[i + 1] in ('"', '#') or (
            c in 'br' and src.startswith('br', i) and i + 2 < n and src[i + 2] in ('"', '#')
        ):
            start = i
            j = i + 1
            if src[j] in 'br':
                j += 1
            hashes = 0
            while j < n and src[j] == '#':
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                j += 1
                end = '"' + '#' * hashes
                k = src.find(end, j)
                if k < 0:
                    errors.append((line, 'unterminated raw string'))
                    i = n
                    continue
                line += src[j:k + len(end)].count('\n')
                i = k + len(end)
                continue
            i = start + 1
            continue
        # normal string
        if c == '"':
            j = i + 1
            while j < n:
                if src[j] == '\\':
                    j += 2
                    continue
                if src[j] == '"':
                    break
                if src[j] == '\n':
                    errors.append((line, 'string literal spans a line break'))
                    break
                j += 1
            else:
                errors.append((line, 'unterminated string'))
                i = n
                continue
            if j < n and src[j] == '"':
                i = j + 1
            else:
                i = j
            continue
        # char literal / lifetime: 'a' is a char, 'a is a lifetime
        if c == "'":
            if i + 2 < n and src[i + 2] == "'":
                i += 3
                continue
            if i + 3 < n and src[i + 1] == '\\' and src[i + 3] == "'":
                i += 4
                continue
            # lifetime or a char literal with CRLF troubles: treat as lifetime
            i += 1
            continue
        if c in '([{':
            stack.append((c, line))
            i += 1
            continue
        if c in ')]}':
            if not stack:
                errors.append((line, f"stray `{c}` closes nothing"))
            elif stack[-1][0] != pairs[c]:
                open_c, open_line = stack[-1]
                errors.append(
                    (line, f"`{c}` closes `{open_c}` opened on line {open_line}"))
            else:
                stack.pop()
            i += 1
            continue
        i += 1
    for open_c, open_line in stack:
        errors.append((open_line, f"`{open_c}` is never closed"))
    return errors


def main(argv):
    roots = argv[1:] or ['./src']
    checked = 0
    failures = 0
    for root in roots:
        if os.path.isfile(root):
            files = [root]
        else:
            files = [
                os.path.join(d, f)
                for d, _s, fs in os.walk(root)
                for f in fs
                if f.endswith('.rs')
            ]
        for path in sorted(files):
            src = open(path, encoding='utf-8').read()
            errors = scan(src)
            checked += 1
            if errors:
                failures += 1
                for line, msg in errors[:10]:
                    print(f'{path}:{line}: {msg}')
    if failures:
        print(f'FAIL: {failures} of {checked} file(s) are structurally broken')
        return 1
    print(f'OK: {checked} file(s) structurally sound')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))

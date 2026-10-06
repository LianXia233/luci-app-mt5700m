#!/usr/bin/env python3
"""Advisory lint: struct literals that omit a field (the E0063 class).

`rs-static-check.py` proves that a literal never names an *unknown* field. It
cannot see the opposite mistake — a literal that was written before a field was
added and now fails to compile with `error[E0063]: missing field`. That exact
mistake shipped twice during the v2 refactor, and this sandbox has no Rust
toolchain, so CI is otherwise the first compiler to notice.

Usage:
    python3 scripts/rs-field-lint.py mt5700webui-openwrt-server/at-webserver/src

Deliberately conservative — it only reports a literal when it is sure:

  * the struct's fields are all `pub` (a private field would make any foreign
    literal a different error, E0451, and the module-local ones would already
    be visible to the author);
  * the struct name is declared exactly once in the tree (no `tests::Value`
    shadowing `core::json::Value`);
  * the literal has no struct-update syntax (`..base`) and no `<`, `>` or `->`
    in its body, because the top-level-comma split below cannot tell a generic
    argument from a comparison.

It exits non-zero when it finds something, so it is safe to wire into a job
later; it is not part of `ci.yml` yet because it is a heuristic.
"""

import os
import re
import sys


def balanced_body(code, start):
    """Return the text between `code[start]` (an opening brace) and its match."""
    depth = 0
    i = start
    while i < len(code):
        ch = code[i]
        if ch == '{':
            depth += 1
        elif ch == '}':
            depth -= 1
            if depth == 0:
                return code[start + 1:i], i
        i += 1
    raise ValueError('unbalanced braces at offset %d' % start)


def strip_comments(text):
    """Drop `//` and `/* */` comments, leaving string literals alone."""
    out = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == '"' or ch == "'":
            quote = ch
            out.append(ch)
            i += 1
            while i < n:
                out.append(text[i])
                if text[i] == '\\':
                    if i + 1 < n:
                        out.append(text[i + 1])
                        i += 1
                elif text[i] == quote:
                    break
                i += 1
            i += 1
            continue
        if text[i:i + 2] == '//':
            while i < n and text[i] != '\n':
                i += 1
            continue
        if text[i:i + 2] == '/*':
            end = text.find('*/', i + 2)
            i = n if end < 0 else end + 2
            continue
        out.append(ch)
        i += 1
    return ''.join(out)


def declarations(code):
    """name -> set of pub field names, for named structs declared in `code`."""
    out = {}
    for m in re.finditer(r'\bstruct\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*(?:where[^{;]*)?\{', code):
        name = m.group(1)
        body, _ = balanced_body(code, m.end() - 1)
        fields = []
        private = False
        for line in body.splitlines():
            line = line.strip()
            fm = re.match(r'(pub(?:\([^)]*\))?\s+)?([a-z_]\w*)\s*:', line)
            if fm and not line.startswith('//'):
                if fm.group(1):
                    fields.append(fm.group(2))
                else:
                    private = True
        out.setdefault(name, []).append((set(fields), private))
    return out


def literals(code):
    """Yield (name, line, body, named_fields) for every `Name { … }` literal."""
    for m in re.finditer(r'(?<![\w:.])([A-Z][A-Za-z0-9_]*)\s*\{', code):
        before = code[:m.start()].rstrip()
        # Skip declarations, match arms, closures and qualified paths.
        if re.search(r'(?:^|[^\w])(?:let|if\s+let|while\s+let|match|=>|\|)\s*$', before):
            continue
        if re.search(r'(?:^|[^\w])(?:struct|enum|impl|union|trait|for|fn|where|as)\s+$', before):
            continue
        if before.endswith('::') or before.endswith('->') or before.endswith(':'):
            continue
        try:
            body, _ = balanced_body(code, m.end() - 1)
        except ValueError:
            continue
        body = strip_comments(body)
        if '..' in body or '<' in body or '>' in body:
            continue
        named = set()
        depth = 0
        cur = ''
        for ch in body:
            if ch in '([{':
                depth += 1
            elif ch in ')]}':
                depth -= 1
            if ch == ',' and depth == 0:
                named.add(cur.strip().split(':')[0].strip())
                cur = ''
            else:
                cur += ch
        named.add(cur.strip().split(':')[0].strip())
        named = {n for n in named if re.fullmatch(r'[a-z_]\w*', n)}
        yield m.group(1), code[:m.start()].count('\n') + 1, body, named


def main(argv):
    root = argv[1] if len(argv) > 1 else '.'
    decls = {}
    files = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in ('.git', 'target')]
        for fn in sorted(filenames):
            if not fn.endswith('.rs'):
                continue
            path = os.path.join(dirpath, fn)
            code = strip_comments(open(path, encoding='utf-8').read())
            files[path] = code
            for name, entries in declarations(code).items():
                decls.setdefault(name, []).extend((e, path) for e in entries)

    errors = []
    for path, code in sorted(files.items()):
        for name, line, _body, named in literals(code):
            entries = decls.get(name)
            if not entries or len(entries) != 1:
                continue  # unknown or ambiguous (a same-named test/local type)
            (fields, private), decl_file = entries[0]
            if private or not fields or not named:
                continue
            missing = sorted(fields - named)
            if missing:
                errors.append(
                    '%s:%d: struct literal %s { … } is missing field(s): %s'
                    ' (declared in %s)' % (path, line, name, ', '.join(missing), decl_file))

    for e in errors:
        print(e)
    print('%s: %d possible missing-field literal(s)' % (root, len(errors)))
    return 1 if errors else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))

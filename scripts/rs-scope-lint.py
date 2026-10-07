#!/usr/bin/env python3
"""Advisory lint: type names used but not in scope (E0412/E0433 class).

The other two cargo-free nets cover structure (`rs-static-check.py`) and
missing struct fields (`rs-field-lint.py`). Neither sees the mistake that cost
this project two CI cycles: a new function whose return type — or a test using
an enum — was never imported, because the *name* was written from memory while
the `use` line was forgotten.

Usage:
    python3 scripts/rs-scope-lint.py mt5700webui-openwrt-server/at-webserver/src

How it decides a name is in scope:

  * declared in the file (`struct`/`enum`/`trait`/`type`/`union`/`const`/
    `static`/`fn`/`mod`);
  * the last segment of a `use` line, including `use a::{b, c as d}` and
    `use a::b::*` (a glob imports *and* marks the file as "cannot prove", which
    is reported separately);
  * a bare generic parameter of the enclosing item (`<T>`, `<K, V>`) — all
    single upper-case letters are accepted as generics;
  * a Rust primitive, prelude item or well-known std type;
  * a local `let`/closure binding, or a struct-literal/enum path already
    resolved by the other lints.

Test modules are skipped: `use super::*;` makes their scope unprovable, and a
test-only import error is what the compiler is best at. The lint reports what
it cannot prove, so it is advisory until a run is clean.
"""

import os
import re
import sys

PRELUDE = {
    'String', 'Vec', 'Option', 'Result', 'Box', 'Some', 'None', 'Ok', 'Err',
    'Self', 'Default', 'Clone', 'Copy', 'Debug', 'PartialEq', 'Eq',
    'PartialOrd', 'Ord', 'Hash', 'Iterator', 'IntoIterator', 'From', 'Into',
    'TryFrom', 'TryInto', 'AsRef', 'AsMut', 'Display', 'Write', 'Read',
    'ToOwned', 'Sized', 'Send', 'Sync', 'Unpin', 'Fn', 'FnMut', 'FnOnce',
    'Drop', 'Deref', 'DerefMut', 'Add', 'Sub', 'Mul', 'Div', 'Index',
    'IndexMut', 'Cow', 'Rc', 'Arc', 'RefCell', 'Cell', 'Mutex', 'RwLock',
    'Duration', 'Instant', 'PathBuf', 'Path', 'OsString', 'HashMap',
    'HashSet', 'BTreeMap', 'BTreeSet', 'VecDeque', 'AtomicBool', 'AtomicU64',
    'AtomicUsize', 'Ordering', 'ParseIntError', 'Boxed', 'Pin', 'TypeId',
    'PhantomData', 'Wrapping', 'Range', 'RangeInclusive',
}
# Single letters are treated as generic parameters (`<T>`, `<K, V>`).
GENERIC_LETTER = re.compile(r'^[A-Z]$')


def strip_code(text):
    """Drop comments and string/char literals, keeping line structure."""
    out = []
    i, n = 0, len(text)
    while i < n:
        if text[i:i + 2] == '//':
            while i < n and text[i] != '\n':
                i += 1
            continue
        if text[i:i + 2] == '/*':
            end = text.find('*/', i + 2)
            if end < 0:
                return ''.join(out)
            out.append('\n' * text.count('\n', i, end))
            i = end + 2
            continue
        ch = text[i]
        # r"…" / r#"…"# / br#"…"# (raw strings have no escapes)
        m = re.match(r'(?:b?r)(#*)"', text[i:])
        if m:
            hashes = '\"' + m.group(1)
            end = text.find(hashes, i + m.end())
            if end < 0:
                return ''.join(out)
            out.append('""')
            out.append('\n' * text.count('\n', i, end))
            i = end + len(hashes)
            continue
        if ch == 'b' and text[i:i + 2] == 'b"':
            i += 1
            ch = '"'
        if ch == '"':
            out.append('""')
            i += 1
            while i < n:
                if text[i] == '\\':
                    i += 2
                    continue
                if text[i] == '"':
                    i += 1
                    break
                if text[i] == '\n':
                    out.append('\n')
                i += 1
            continue
        if ch == "'":
            m = re.match(r"'(?:\\u\{[0-9a-fA-F]+\}|\\.|[^\\'])'", text[i:])
            if m:
                i += m.end()
                out.append("''")
                continue
            # a lifetime like 'static / 'a
            m = re.match(r"'[a-z_]\w*", text[i:])
            if m:
                i += m.end()
                continue
        out.append(ch)
        i += 1
    return ''.join(out)


def split_test_module(code):
    """(top-level code, test-module code) — the test module is usually the last
    item, so everything from its attribute on counts as test code."""
    idx = code.find('#[cfg(test)]')
    if idx < 0:
        return code, ''
    return code[:idx], code[idx:]


def imported_names(code):
    """(names, unprovable) — `unprovable` means a glob that is not `super::*`.

    `use super::*;` is how every unit-test module inherits the file's scope, and
    the caller adds that scope explicitly, so it must not hide the file."""
    names = set()
    glob = False
    for m in re.finditer(r'(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+([^;]+);', code):
        path = m.group(1)
        if path.rstrip().endswith('::*'):
            if path.strip() not in ('super::*', 'self::*'):
                glob = True
            continue
        brace = re.search(r'\{([^}]*)\}', path)
        if brace:
            for part in brace.group(1).split(','):
                part = part.strip()
                if not part:
                    continue
                if part == 'self':
                    base = path[:brace.start()].rstrip(': ')
                    names.add(base.split('::')[-1])
                    continue
                if '*' in part:
                    glob = True
                    continue
                # `state::QosState` -> QosState, `commands as qos_commands` -> qos_commands
                name = part.split(' as ')[-1].strip().split('::')[-1].strip()
                if name:
                    names.add(name)
            continue
        last = path.split('::')[-1].strip()
        if ' as ' in last:
            last = last.split(' as ')[-1].strip()
        if last and last != '*':
            names.add(last)
    return names, glob


def declared_names(code):
    names = set()
    for m in re.finditer(
        r'\b(?:pub(?:\([^)]*\))?\s+)?(?:const\s+|unsafe\s+|async\s+)*'
        r'(?:fn|struct|enum|trait|type|union|const|static|mod)\s+([A-Za-z_]\w*)', code):
        names.add(m.group(1))
    # Variants of a `enum` declared in this file are used unqualified.
    for m in re.finditer(r'\benum\s+[A-Za-z_]\w*[^{]*\{', code):
        depth, i = 0, m.end() - 1
        start = i
        while i < len(code):
            if code[i] == '{':
                depth += 1
            elif code[i] == '}':
                depth -= 1
                if depth == 0:
                    break
            i += 1
        body = code[start:i]
        for vm in re.finditer(r'(?m)^\s*([A-Z][A-Za-z0-9_]*)\s*(?:\(|\{|,|$)', body):
            names.add(vm.group(1))
    return names


def used_type_names(code):
    """Capitalised identifiers that are not `Path::Qualified` or `.member`s."""
    used = {}
    for m in re.finditer(r"(?<![:\w.'\"])([A-Z][A-Za-z0-9_]*)", code):
        name = m.group(1)
        line = code[:m.start()].count('\n') + 1
        used.setdefault(name, line)
    return used


def main(argv):
    root = argv[1] if len(argv) > 1 else '.'
    findings = []
    glob_files = []
    files = 0
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in ('.git', 'target')]
        for fn in sorted(filenames):
            if not fn.endswith('.rs'):
                continue
            path = os.path.join(dirpath, fn)
            code = strip_code(open(path, encoding='utf-8').read())
            top, tests = split_test_module(code)
            if not top.strip():
                continue
            files += 1
            names, glob = imported_names(top)
            if glob:
                glob_files.append(path)
                continue
            top_scope = names | declared_names(top) | PRELUDE
            # The unit-test module inherits the file's scope through `use super::*`
            # (and is checked with it, which is what catches a test using an enum
            # the file never imported — the second half of this lint's job).
            if tests.strip():
                t_names, t_glob = imported_names(tests)
                in_scope = top_scope | t_names | declared_names(tests)
                if t_glob:
                    glob_files.append(path)
                for name, line in sorted(used_type_names(tests).items(), key=lambda kv: kv[1]):
                    if name in in_scope or GENERIC_LETTER.match(name):
                        continue
                    findings.append('%s:%d: %s is not in scope (no declaration, import or prelude entry)' % (path, line, name))
            for name, line in sorted(used_type_names(top).items(), key=lambda kv: kv[1]):
                if name in top_scope or GENERIC_LETTER.match(name):
                    continue
                findings.append('%s:%d: %s is not in scope (no declaration, import or prelude entry)' % (path, line, name))
    for f in findings:
        print(f)
    print('%d file(s), %d not-in-scope name(s), %d file(s) skipped (glob import)' % (files, len(findings), len(glob_files)))
    return 1 if findings else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))

#!/usr/bin/env python3
"""Static checks for the std-only Rust backend (`cargo`-free safety net).

There is no rustc in this sandbox, so this script stands in for the compiler on
the one class of error a mechanical refactor produces most often: a path that no
longer resolves (`crate::state::collectors::spawn_all` after the file was split)
or a function called with the wrong number of arguments.

It is a *resolver*, not a type checker: it walks the real `mod` tree, collects
every declared item with its visibility and arity, follows `pub use` re-exports,
then checks every `crate::…` path in every file. Callers are only checked for
arity when the callee resolves to a single unambiguous `fn`.

Checked, in order:
  1. every `mod x;` has a file;
  2. every `crate::…` path resolves through the module tree;
  3. `crate::path::f(…)` calls pass the argument count `f` declares;
  4. bare `f(…)` calls resolve locally (catches a deleted/renamed function that
     is still referenced somewhere else in the crate);
  5. every `impl Trait for Type` defines the trait's required methods (catches
     the "trait gained a method" class of breakage);
  6. struct literals only name fields the struct declares;
  7. one item never carries two `#[derive(...)]` attributes, which is how a
     scripted insert that lands between an attribute and its item shows up
     (the item below silently loses `Debug`/`Default`/`PartialEq`);
  8. a module-qualified call (`commands::print_qos(…)`) is only used in a file
     that actually imports that module — a moved line that kept the call but
     lost the `use` compiles nowhere (this is how a scripted edit broke
     `modules/network/api.rs` once);
  9. two closures in one function never mutate the same local variable while
     both are live (`put_str` and `put_num` both calling `m.insert(…)`): the
     borrow checker rejects it, and it is how a JSON-rendering helper written
     in the two-closure style breaks the build.

Points 2-6 mirror the compiler errors a mechanical refactor produces most
often, which is why this runs in CI next to `cargo test` (the Rust toolchain is
not always available where the refactor is reviewed, and CI logs are not always
reachable — see scripts/ci-annotate-cargo.py).

Usage:  python3 scripts/rs-static-check.py <crate-src-dir>   (default: ./src)
Exit code 1 when anything fails to resolve.
"""
import os
import re
import sys

KEYWORDS = {
    'self', 'super', 'crate', 'Self', 'true', 'false', 'fn', 'let', 'mut', 'ref',
    'if', 'else', 'match', 'while', 'for', 'in', 'return', 'break', 'continue',
    'struct', 'enum', 'impl', 'trait', 'use', 'pub', 'mod', 'const', 'static',
    'type', 'as', 'where', 'move', 'async', 'await', 'dyn', 'unsafe', 'loop',
    'box', 'crate', 'extern', 'unsized', 'union',
}

# ---------------------------------------------------------------- lexing


def strip_code(src):
    """Replace comments and string/char literals with blanks, keeping newlines.

    Lifetimes (`'a`) and char literals (`'\n'`, `'}'`, `'x'`) must survive as
    code or brace counting breaks, so this is a small hand-written scanner
    rather than a regex.
    """
    out = []
    i, n = 0, len(src)
    prev = ''
    while i < n:
        c = src[i]
        if c == '/' and i + 1 < n and src[i + 1] == '/':
            while i < n and src[i] != '\n':
                i += 1
            continue
        if c == '/' and i + 1 < n and src[i + 1] == '*':
            depth = 1
            i += 2
            while i < n and depth:
                if src[i] == '\n':
                    out.append('\n')
                elif src.startswith('/*', i):
                    depth += 1
                    i += 1
                elif src.startswith('*/', i):
                    depth -= 1
                    i += 1
                i += 1
            continue
        if c == '"':
            # raw strings r"..." / r#"..."#
            j = i - 1
            while j >= 0 and src[j] in 'r#':
                j -= 1
            prev_nonspace = src[j] if j >= 0 else ''
            hashes = 0
            if prev_nonspace == 'r':
                k = j + 1
                while k < n and src[k] == '#':
                    hashes += 1
                    k += 1
                if k < n and src[k] == '"':
                    term = '"' + '#' * hashes
                    i = k + 1
                    while i < n and not src.startswith(term, i):
                        out.append('\n' if src[i] == '\n' else '~')
                        i += 1
                    i += len(term)
                    out.append(' ' * len(term))
                    continue
            i += 1
            while i < n:
                if src[i] == '\\':
                    out.append('~~')
                    i += 2
                    continue
                if src[i] == '"':
                    i += 1
                    break
                # '~' (not a space) so an argument that is only a string still
                # counts as an argument in arity checks.
                out.append('\n' if src[i] == '\n' else '~')
                i += 1
            out.append('~')
            prev = '"'
            continue
        if c == "'":
            # lifetime = 'ident (not followed by a closing quote), else char lit
            m = re.match(r"'[A-Za-z_][A-Za-z0-9_]*", src[i:])
            if m and not src[i + len(m.group(0)):i + len(m.group(0)) + 1] == "'":
                out.append(m.group(0))
                i += len(m.group(0))
                prev = 'l'
                continue
            m = re.match(r"'(?:\\.|[^\\'])'", src[i:])
            if m:
                out.append(' ' * len(m.group(0)))
                i += len(m.group(0))
                continue
            # lone quote (char literal we failed to match): keep it, harmless
            out.append(c)
            i += 1
            continue
        out.append(c)
        i += 1
    return ''.join(out)


def arg_count(argstr):
    """Count top-level comma separated arguments, ignoring trailing commas."""
    argstr = argstr.strip()
    if not argstr:
        return 0
    depth = 0
    pieces = []
    cur = ''
    for ch in argstr:
        if ch in '([{':
            depth += 1
        elif ch in ')]}':
            depth -= 1
        if ch == ',' and depth == 0:
            pieces.append(cur)
            cur = ''
        else:
            cur += ch
    pieces.append(cur)
    return len([p for p in pieces if p.strip()])


# ---------------------------------------------------------------- module tree


class Mod:
    __slots__ = ('path', 'file', 'items', 'children', 'reexports', 'parent')

    def __init__(self, path, file=None, parent=None):
        self.path = path          # crate-relative path tuple
        self.file = file
        self.items = {}           # name -> dict(kind, vis, arity, file, line)
        self.children = {}
        self.reexports = {}       # alias -> absolute crate path tuple
        self.parent = parent


def balanced_body(code, start):
    """Return (body, end_index) for the brace block starting at `start`."""
    depth = 0
    i = start
    while i < len(code):
        if code[i] == '{':
            depth += 1
        elif code[i] == '}':
            depth -= 1
            if depth == 0:
                return code[start + 1:i], i
        i += 1
    return code[start + 1:], len(code)


def collect_items(body, mod, file, base_line):
    """Collect item declarations from a module body (one nesting level deep)."""
    i = 0
    n = len(body)
    while i < n:
        m = re.compile(
            r'(?m)^[ \t]*(pub(?:\s*\((?:crate|super|in\s+[^)]*)\))?\s+)?'
            r'(fn|struct|enum|trait|type|const|static|union)\s+([A-Za-z_][A-Za-z0-9_]*)'
        ).search(body, i)
        if not m:
            break
        vis = 'pub' if m.group(1) and 'pub' in m.group(1) else 'priv'
        kind, name = m.group(2), m.group(3)
        after = m.end()
        arity = None
        if kind == 'fn':
            p = after
            while p < n and body[p] in ' \t\n':
                p += 1
            if p < n and body[p] == '<':          # generics
                depth = 0
                while p < n:
                    if body[p] == '<':
                        depth += 1
                    elif body[p] == '>':
                        depth -= 1
                        if depth == 0:
                            p += 1
                            break
                    p += 1
            while p < n and body[p] in ' \t':
                p += 1
            if p < n and body[p] == '(':
                args, end = balanced_body(body, p).__class__((None, None)) if False else (None, None)
                # balanced_body works on braces; do parens manually
                depth, q = 0, p
                while q < n:
                    if body[q] == '(':
                        depth += 1
                    elif body[q] == ')':
                        depth -= 1
                        if depth == 0:
                            break
                    q += 1
                argstr = body[p + 1:q]
                args = []
                d2 = 0
                cur = ''
                for ch in argstr:
                    if ch in '([{':
                        d2 += 1
                    elif ch in ')]}':
                        d2 -= 1
                    if ch == ',' and d2 == 0:
                        args.append(cur)
                        cur = ''
                    else:
                        cur += ch
                if cur.strip():
                    args.append(cur)
                args = [a for a in args if a.strip()]
                arity = len(args)
                first = args[0].strip() if args else ''
                if re.match(r'^(?:&\s*)?(?:mut\s+)?self\b', first) or first in ('self', '&self', '&mut self'):
                    arity -= 1
        line = base_line + body[:m.start()].count('\n')
        existing = mod.items.get(name)
        if existing is None or (existing['vis'] == 'priv' and vis == 'pub'):
            mod.items[name] = {'kind': kind, 'vis': vis, 'arity': arity,
                               'file': file, 'line': line}
        i = m.end()


def parse_module(mod, file, code, base_line=1):
    """Parse one module body: item decls, child mods, re-exports."""
    code = strip_code(code)
    collect_items(code, mod, file, base_line)

    # re-exports: `pub use crate::a::b::{X, Y as Z};`
    for m in re.finditer(r'(?m)^[ \t]*pub\s+use\s+([^;]+);', code):
        raw = ' '.join(m.group(1).split())
        for piece in re.split(r',(?![^<{]*[}>])', raw):
            piece = piece.strip()
            if not piece:
                continue
            mm = re.match(r'(crate(?:::\w+)+)(?:::\{([^}]*)\})?$', piece)
            if mm:
                prefix = tuple(mm.group(1).split('::')[1:])
                inner = mm.group(2)
                if inner is None:
                    if prefix:
                        mod.reexports[prefix[-1]] = prefix
                    continue
                for name in inner.split(','):
                    name = name.strip()
                    if not name:
                        continue
                    alias = name.split(' as ')[-1].strip()
                    leaf = name.split(' as ')[0].strip()
                    mod.reexports[alias] = prefix + (leaf,)

    # child modules
    for m in re.finditer(r'(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;', code):
        name = m.group(1)
        child_path = mod.path + (name,)
        child_file = resolve_mod_file(file, name)
        child = Mod(child_path, child_file, mod)
        mod.children[name] = child
        if child_file:
            load_module(child, child_file)
        else:
            MISSING.append((file, base_line + code[:m.start()].count('\n'), name,
                            tuple(child_path)))

    for m in re.finditer(r'(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*\{', code):
        name = m.group(1)
        body, _ = balanced_body(code, m.end() - 1)
        child = Mod(mod.path + (name,), file, mod)
        mod.children[name] = child
        parse_module(child, file, body, base_line + code[:m.end()].count('\n'))


def resolve_mod_file(parent_file, name):
    d = os.path.dirname(parent_file)
    if os.path.basename(parent_file) in ('mod.rs',) or True:
        cands = [os.path.join(d, name + '.rs'), os.path.join(d, name, 'mod.rs')]
    for c in cands:
        if os.path.exists(c):
            return c
    return None


def load_module(mod, file):
    code = open(file, encoding='utf-8').read()
    parse_module(mod, file, code)


# ---------------------------------------------------------------- resolution


def find_child(mod, seg):
    if seg in mod.children:
        return mod.children[seg]
    if seg == 'super' and mod.parent:
        return mod.parent
    return None


def resolve_path(root, from_mod, segs):
    """Resolve a crate-relative path. Returns (kind, module, leaf_info) or None."""
    cur = root
    i = 0
    while i < len(segs):
        seg = segs[i]
        if seg == 'super':
            cur = cur.parent or cur
            i += 1
            continue
        child = cur.children.get(seg)
        if child is not None:
            cur = child
            i += 1
            continue
        if seg in cur.items:
            return ('item', cur, cur.items[seg], tuple(segs[i + 1:]))
        if seg in cur.reexports:
            target = cur.reexports[seg]
            res = resolve_path(root, root, target)
            if res and i + 1 < len(segs):
                # re-exported type followed by ::method — accept
                return ('item', res[1], res[2], tuple(segs[i + 1:]))
            return res
        if seg in ('std', 'core', 'alloc', 'self', 'Self'):
            return ('external', cur, None, ())
        return None
    return ('module', cur, None, ())


def split_top_commas(text):
    """Split `a::{b, c::{d, e}}`'s inner text on the top-level commas."""
    parts, buf, depth = [], '', 0
    for ch in text:
        if ch == '{':
            depth += 1
        elif ch == '}':
            depth -= 1
        if ch == ',' and depth == 0:
            parts.append(buf)
            buf = ''
        else:
            buf += ch
    if buf.strip():
        parts.append(buf)
    return parts


def expand_use_tree(prefix, text):
    """Expand a `use` tree into full paths: `a::{b, c::{d, e}}` -> four paths.

    `crate::` is kept in the returned strings so the caller can resolve them
    with the same resolver as every other path.
    """
    text = re.sub(r'\s+as\s+[A-Za-z_]\w*\s*$', '', text.strip()).strip()
    if not text:
        return []
    if '{' in text:
        head, rest = text.split('{', 1)
        inner, tail = rest.rsplit('}', 1)
        head = head.rstrip(':').strip()
        base = '::'.join(p for p in (prefix, head) if p)
        out = []
        for part in split_top_commas(inner):
            out.extend(expand_use_tree(base, part + tail))
        return out
    full = '::'.join(p for p in (prefix, text) if p)
    if not full:
        return []
    if full.endswith('::*'):
        full = full[:-3]
    if full.endswith('::self'):
        full = full[:-6]
    return [full] if full else []


USE_CRATE = re.compile(
    r'(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?use\s+(crate(?:::[^;]*)?)\s*;')


def check_use_paths(root, mod, code, path, errors):
    """Every `use crate::…` path must resolve — braces and `as` included.

    `crate::core::task::TaskCtx` after the scheduler split is the exact mistake
    this catches: `crate::core::task` alone resolves, so a plain path scan sees
    nothing wrong while rustc stops at the leaf.
    """
    for m in USE_CRATE.finditer(code):
        tree = m.group(1)
        if not tree.startswith('crate'):
            continue
        for full in expand_use_tree('', tree):
            segs = tuple(full.split('::')[1:])
            if not segs:
                continue
            if resolve_path(root, mod, segs) is None:
                line = code[:m.start()].count('\n') + 1
                errors.append(f'{path}:{line}: unresolved `use {full}`')


MISSING = []

CRATE_PATH = re.compile(r'\bcrate((?:::[A-Za-z_]\w*)+)')

# `crate::a::b::f(` call sites for arity checking
CALL = re.compile(r'\bcrate((?:::[A-Za-z_]\w*)+)\s*(\()')


def check(root, src_dir):
    errors = []
    checked = 0
    for dirpath, _dirs, files in os.walk(src_dir):
        for f in files:
            if not f.endswith('.rs'):
                continue
            path = os.path.join(dirpath, f)
            mod = find_mod_for_file(root, path)
            code = strip_code(open(path, encoding='utf-8').read())
            check_use_paths(root, mod, code, path, errors)
            for m in CRATE_PATH.finditer(code):
                segs = tuple(m.group(1).split('::')[1:])
                res = resolve_path(root, mod, segs)
                checked += 1
                if res is None:
                    line = code[:m.start()].count('\n') + 1
                    errors.append(f'{path}:{line}: unresolved crate::{"::".join(segs)}')
                elif res[0] == 'item' and res[3]:
                    # item::rest — allow methods/variants/assoc fns/fields
                    rest = res[3]
                    info = res[2]
                    if info['kind'] in ('struct', 'enum', 'trait', 'union', 'type'):
                        continue
                    if info['kind'] in ('const', 'static'):
                        line = code[:m.start()].count('\n') + 1
                        errors.append(
                            f'{path}:{line}: crate::{"::".join(segs)} — value used as a path')
                    elif info['kind'] == 'fn':
                        continue
            for m in CALL.finditer(code):
                segs = tuple(m.group(1).split('::')[1:])
                res = resolve_path(root, mod, segs)
                if not (res and res[0] == 'item' and not res[3] and res[2]['arity'] is not None):
                    continue
                # collect call args
                depth, q, start = 0, m.end() - 1, m.end() - 1
                while q < len(code):
                    if code[q] == '(':
                        depth += 1
                    elif code[q] == ')':
                        depth -= 1
                        if depth == 0:
                            break
                    q += 1
                given = arg_count(code[start + 1:q])
                want = res[2]['arity']
                if given != want:
                    line = code[:m.start()].count('\n') + 1
                    errors.append(
                        f'{path}:{line}: crate::{"::".join(segs)} expects {want} arg(s), '
                        f'call passes {given}')
    return checked, errors


def find_mod_for_file(mod, path):
    if mod.file == path:
        return mod
    for child in mod.children.values():
        found = find_mod_for_file(child, path)
        if found:
            return found
    return mod



# ------------------------------------------------- local call check

def bound_names(code):
    """Identifiers introduced by `use`/`let`/params inside one file body."""
    names = set()
    for m in re.finditer(r'(?m)^[ \t]*(?:pub[^;]*?)?use\s+([^;]+);', code):
        raw = m.group(1)
        # Expand every brace group: `a::{b, c::{d, e}}` -> full paths.
        pieces = []
        stack = [[]]
        cur = ''
        for ch in raw:
            if ch == '{':
                stack.append([])
                cur = cur.rstrip(':').strip()
                if cur:
                    stack[-2].append(cur + '::')
                cur = ''
            elif ch == '}':
                if cur.strip():
                    stack[-1].append(cur.strip())
                cur = ''
                group = stack.pop()
                prefix = ''
                items = group
                if group and group[0].endswith('::'):
                    prefix = group[0]
                    items = group[1:]
                merged = prefix + '{' + ','.join(items) + '}'
                if stack:
                    stack[-1].append(merged)
                else:
                    pieces.append(merged)
            elif ch == ',':
                if cur.strip():
                    stack[-1].append(cur.strip())
                cur = ''
            else:
                cur += ch
        if cur.strip():
            stack[-1].append(cur.strip())
        if not pieces:
            pieces = stack[0]
        # Recursively flatten the merged groups.
        def flatten(pieces, out):
            for piece in pieces:
                if '{' in piece:
                    prefix, rest = piece.split('{', 1)
                    inner = rest.rsplit('}', 1)[0]
                    sub = []
                    depth = 0
                    buf = ''
                    for ch in inner:
                        if ch == '{':
                            depth += 1
                        elif ch == '}':
                            depth -= 1
                        if ch == ',' and depth == 0:
                            sub.append(buf)
                            buf = ''
                        else:
                            buf += ch
                    if buf:
                        sub.append(buf)
                    flatten([prefix + s.strip() for s in sub if s.strip()], out)
                else:
                    out.append(piece)
        flat = []
        flatten(pieces, flat)
        for piece in flat:
            piece = piece.strip()
            if not piece:
                continue
            piece = piece.split(' as ')[-1].strip()
            if piece in ('self', ''):
                continue
            names.add(piece.split('::')[-1])
    for m in re.finditer(r'\blet\s+(?:mut\s+)?([A-Za-z_]\w*)', code):
        names.add(m.group(1))
    for m in re.finditer(r'\blet\s+(?:mut\s+)?\(?([^=;\n]*)', code):
        for part in m.group(1).split(','):
            part = part.strip().strip('()')
            mm = re.match(r'(?:mut\s+)?([A-Za-z_]\w*)', part)
            if mm:
                names.add(mm.group(1))
    for m in re.finditer(r'\blet\s+(?:mut\s+)?[A-Za-z_]\w*(?:::\s*<[^>]*>)?\s*\{\s*([^}]*)\}', code):
        for part in m.group(1).split(','):
            part = part.strip()
            if not part:
                continue
            alias = part.split(':')[-1].strip()
            mm = re.match(r'(?:mut\s+)?([A-Za-z_]\w*)', alias)
            if mm:
                names.add(mm.group(1))
    for m in re.finditer(r'\bfor\s+\(?([^)\n]*?)\)?\s+in\b', code):
        for part in m.group(1).split(','):
            mm = re.match(r'\s*(?:mut\s+|&\s*)*(?:mut\s+)?([A-Za-z_]\w*)', part)
            if mm:
                names.add(mm.group(1))
    for m in re.finditer(r'\|([^|]*)\|', code):
        for part in re.split(r'[,\s]+', m.group(1)):
            part = part.strip().lstrip('&').replace('mut ', '')
            if re.fullmatch(r'[A-Za-z_]\w*', part):
                names.add(part)
    for m in re.finditer(
        r'\b(?:(?:const|async|unsafe|pub|extern)\s+)*fn\s+([A-Za-z_]\w*)', code
    ):
        names.add(m.group(1))
    for m in re.finditer(
        r'\b(?:struct|enum|trait|type|const|static|mod|union)\s+([A-Za-z_]\w*)', code
    ):
        names.add(m.group(1))
    return names


def check_local_calls(root, src_dir):
    """Flag calls to crate functions that were deleted/renamed locally."""
    declared = {}
    files = {}
    for dirpath, _d, fs in os.walk(src_dir):
        for f in fs:
            if not f.endswith('.rs'):
                continue
            path = os.path.join(dirpath, f)
            code = strip_code(open(path, encoding='utf-8').read())
            files[path] = code
            for m in re.finditer(r'\bfn\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*\(([^)]*)', code):
                params = m.group(2).strip()
                if params.startswith('self') or params.startswith('&self') or params.startswith('&mut self') or params.startswith('mut self'):
                    continue  # trait/impl method, never called by bare name
                declared.setdefault(m.group(1), path)
    errors = []
    for path, code in files.items():
        local = bound_names(code)
        for m in re.finditer(r'(?<![\w.:])([a-z_][a-z0-9_]*)\s*\(', code):
            name = m.group(1)
            if name in local:
                continue
            if name not in declared:
                continue
            # `#[cfg(...)]` style attributes are not calls
            before = code[:m.start()].rstrip()
            if before.endswith('#'):
                continue
            # macro invocations (`assert_eq!(...)`) are not function calls
            if code[m.end() - 1:m.end() + 1] == '!':
                continue
            if name in ('cfg', 'derive', 'test', 'inline', 'allow', 'serde'):
                continue
            line = code[:m.start()].count('\n') + 1
            errors.append(
                f'{path}:{line}: call to `{name}()` — defined in '
                f'{os.path.relpath(declared[name], src_dir)} but not in scope here')
    return errors



# ------------------------------------------------- semantic checks


def struct_fields(code):
    """Map struct/enum name -> (kind, set of fields/variants, has_private)."""
    out = {}
    for m in re.finditer(r'\bstruct\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*(?:where[^{]*)?\{', code):
        name = m.group(1)
        body, _ = balanced_body(code, m.end() - 1)
        out[name] = ('struct', set(re.findall(r'(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?([a-z_]\w*)\s*:', body)), True)
    for m in re.finditer(r'\bstruct\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*\(([^;]*)\);', code):
        out[m.group(1)] = ('tuple', set(), False)
    for m in re.finditer(r'\benum\s+([A-Za-z_]\w*)[^{]*\{', code):
        body, _ = balanced_body(code, m.end() - 1)
        variants = set()
        for vm in re.finditer(r'(?m)^[ \t]*([A-Za-z_]\w*)\s*(?:\(|\{|,|$)', body):
            variants.add(vm.group(1))
        out[m.group(1)] = ('enum', variants, False)
    return out


def check_traits(declared_traits, impls, src_dir):
    """Every `impl Trait for Type` must define the trait's required methods."""
    errors = []
    for path, trait_name, line, methods in impls:
        info = declared_traits.get(trait_name)
        if info is None:
            continue
        required = info['required']
        missing = sorted(required - methods)
        if missing:
            errors.append(
                f'{path}:{line}: impl {trait_name} is missing required method(s): '
                + ', '.join(missing))
    return errors


def collect_traits(src_dir):
    traits = {}
    impls = []
    files = {}
    for dirpath, _d, fs in os.walk(src_dir):
        for f in fs:
            if not f.endswith('.rs'):
                continue
            path = os.path.join(dirpath, f)
            code = strip_code(open(path, encoding='utf-8').read())
            files[path] = code
            for m in re.finditer(r'\btrait\s+([A-Za-z_]\w*)[^{;]*\{', code):
                body, _ = balanced_body(code, m.end() - 1)
                required = set()
                for fm in re.finditer(r'\bfn\s+([a-z_]\w*)\s*(?:<[^>]*>)?\s*\(', body):
                    # a declaration ends with `;` (required), a default body with `{`
                    rest = body[fm.end():]
                    depth, i = 1, 0
                    while i < len(rest):
                        if rest[i] == '(':
                            depth += 1
                        elif rest[i] == ')':
                            depth -= 1
                            if depth == 0:
                                break
                        i += 1
                    after = rest[i + 1:]
                    j = 0
                    while j < len(after) and after[j] not in '{;':
                        j += 1
                    if j < len(after) and after[j] == ';':
                        required.add(fm.group(1))
                traits.setdefault(m.group(1), {'required': set(), 'file': path})['required'] |= required
            for m in re.finditer(r'\bimpl(?:\s*<[^>]*>)?\s+([A-Za-z_]\w*)\s+for\s+([A-Za-z_]\w*)', code):
                body, _ = balanced_body(code, m.end())
                methods = set(re.findall(r'\bfn\s+([a-z_]\w*)', body))
                line = code[:m.start()].count('\n') + 1
                impls.append((path, m.group(1), line, methods))
    return traits, impls, files


def check_struct_literals(files):
    """Fields named in a struct literal must exist on that struct."""
    all_structs = {}
    for path, code in files.items():
        for name, info in struct_fields(code).items():
            if info[0] == 'struct':
                all_structs.setdefault(name, (info[1], path))
    errors = []
    for path, code in files.items():
        for m in re.finditer(r'(?<![\w:.])([A-Z][A-Za-z0-9_]*)\s*\{', code):
            name = m.group(1)
            if name not in all_structs:
                continue
            fields, decl_file = all_structs[name]
            line = code[:m.start()].count('\n') + 1
            # skip patterns: `let X {`, `if let X {`, match arms, closures
            before = code[:m.start()].rstrip()
            if re.search(r'(?:^|[^\w])(?:let|if\s+let|while\s+let|match|=>|\|)\s*$', before):
                continue
            if re.search(r'(?:^|[^\w])(?:struct|enum|impl|union|trait|for|fn|where)\s+$', before + ' '):
                continue
            if re.search(r'(?:^|[^\w])(?:struct|enum|impl|union|trait)\s+[\w<>, ]*$', before):
                continue
            body, _ = balanced_body(code, m.end() - 1)
            # Struct-update syntax (`..base`) is allowed, but the fields named
            # *before* it still have to exist — only the base's own fields are
            # unknown to this check, so cut the body at the first top-level `..`.
            named = set()
            depth = 0
            cur = ''
            i = 0
            while i < len(body):
                ch = body[i]
                if ch == '.' and body[i:i + 2] == '..' and depth == 0:
                    break
                i += 1
                if ch in '([{<':
                    depth += 1
                elif ch in ')]}>':
                    depth -= 1
                if ch == ',' and depth == 0:
                    named.add(cur.strip().split(':')[0].strip())
                    cur = ''
                else:
                    cur += ch
            named.add(cur.strip().split(':')[0].strip())
            named = {n for n in named if re.fullmatch(r'[a-z_]\w*', n)}
            unknown = sorted(named - fields)
            if unknown:
                errors.append(
                    f'{path}:{line}: struct literal {name} {{ … }} uses unknown field(s): '
                    + ', '.join(unknown))
    return errors



def check_enum_variants(files):
    """`Enum::Variant` paths must name a variant the enum declares."""
    enums = {}
    for path, code in files.items():
        for m in re.finditer(r'\benum\s+([A-Za-z_]\w*)[^{;]*\{', code):
            body, _ = balanced_body(code, m.end() - 1)
            variants = set()
            for vm in re.finditer(r'(?m)^[ \t]*([A-Za-z_]\w*)\s*(?:\(|\{|,|$|=)', body):
                variants.add(vm.group(1))
            enums.setdefault(m.group(1), (variants, path))
    errors = []
    for path, code in files.items():
        for m in re.finditer(r'(?<![\w:.])([A-Z][A-Za-z0-9_]*)::([A-Z][A-Za-z0-9_]*)\s*(?:\(|\{|,|\)|;|\.)', code):
            name, variant = m.group(1), m.group(2)
            if name not in enums:
                continue
            variants, decl = enums[name]
            # Self:: is resolved by the compiler against the surrounding impl
            if variant in variants:
                continue
            line = code[:m.start()].count('\n') + 1
            errors.append(
                f'{path}:{line}: {name}::{variant} — enum {name} '
                f'({os.path.relpath(decl, os.path.dirname(path))}) has no such variant')
    return errors



def check_split_derives(files):
    """Two `#[derive(...)]` attributes on one item mean a scripted insert landed
    between an item's attribute and the item — the second derive (and any doc
    comment in between) actually belongs to the *next* declaration, which then
    silently loses `Debug`/`Default`/`PartialEq` and produces a wall of E0277/
    E0599 errors in a file the insert never meant to touch.
    """
    errors = []
    for path, code in files.items():
        lines = code.split('\n')
        for i, line in enumerate(lines):
            if not re.match(r'\s*(pub\s+)?(struct|enum|union)\b', line):
                continue
            derives = 0
            j = i - 1
            while j >= 0:
                stripped = lines[j].strip()
                if stripped.startswith('#[') or stripped.startswith('///') or stripped.startswith('//!'):
                    if stripped.startswith('#[derive'):
                        derives += 1
                    j -= 1
                    continue
                # blank lines are skipped: `strip_code` blanks every comment, so
                # doc comments (and therefore the gap they leave) must not end
                # the block — otherwise a stacked derive is invisible.
                if not stripped:
                    j -= 1
                    continue
                break
            if derives > 1:
                errors.append(
                    f'{path}:{i + 1}: {derives} derive attributes stacked on one item '
                    f'(a declaration lost its own derives?)')
    return errors


# Sibling module names a file may call into without a `crate::` prefix. Kept
# deliberately short: these are the file-per-concern names every module uses,
# so a bare `commands::` / `parser::` / `state::` reference almost always means
# "this file imported the module" — and when it did not, the compiler would
# reject it. (`json::`, `parse::<…>` and friends are covered by the crate::-
# path checker for qualified uses and are not listed here.)
MODULE_QUALIFIERS = (
    'commands', 'parser', 'state', 'service', 'api', 'channel', 'cache', 'bus',
    'registry', 'refresh', 'jobs', 'arbiter', 'gate', 'plan', 'reports',
)

IMPORT_NAME_RE = re.compile(r'\b(?:use|pub\s+use)\b([^;]*);')


def imported_names(code):
    """Names a file's `use` items bring into scope, plus whether it globs.

    Returns `(names, has_glob)`; a glob import that is not `super::*` makes the
    check skip the file, because anything could be in scope.
    """
    names = set()
    has_glob = False
    for m in IMPORT_NAME_RE.finditer(code):
        spec = m.group(1).strip()
        if spec.startswith('super::*') or spec == '*':
            continue
        if '*' in spec:
            has_glob = True
            continue
        if '{' in spec:
            head, _, tail = spec.partition('{')
            head = head.strip().rstrip(':')
            for part in tail.rstrip('}').split(','):
                part = part.strip()
                if not part:
                    continue
                if part == 'self':
                    names.add(head.split('::')[-1])
                    continue
                name = part.split(' as ')[-1].strip() if ' as ' in part else part
                names.add(name)
            continue
        spec = spec.split(' as ')[-1].strip() if ' as ' in spec else spec
        names.add(spec.split('::')[-1])
    return names, has_glob


def declared_module_names(code):
    """Child module names this file declares (`mod x;` / `pub mod x;`).

    A declaration puts the name in scope exactly like an import does, which is
    why `main.rs` can write `api::cli::run(…)` after `mod api;`.
    """
    return set(re.findall(r'(?m)^\s*(?:pub\s+)?mod\s+([A-Za-z_]\w*)\s*;', code))


def use_statement_lines(lines):
    """Line indexes that belong to a `use` item (single- or multi-line)."""
    skip = set()
    in_use = False
    for i, line in enumerate(lines):
        if not in_use and re.match(r'\s*(?:pub\s+)?use\b', line):
            in_use = True
        if in_use:
            skip.add(i)
            if line.rstrip().endswith(';'):
                in_use = False
    return skip


def check_bare_module_paths(files):
    """`commands::f(…)` in a file that never imported `commands`."""
    errors = []
    for path in sorted(files):
        code = open(path, encoding='utf-8').read()
        stripped = strip_code(code)
        names, has_glob = imported_names(stripped)
        names |= declared_module_names(stripped)
        if has_glob:
            continue
        lines = stripped.split('\n')
        skip = use_statement_lines(lines)
        for i, line in enumerate(lines):
            if i in skip:
                continue
            for m in re.finditer(r'(?<![:\w])([A-Za-z_]\w*)::', line):
                name = m.group(1)
                if name in MODULE_QUALIFIERS and name not in names:
                    errors.append(
                        f'{path}:{i + 1}: `{name}::` is used but `{name}` is not imported '
                        f'in this file')
    return errors


# Local mutators a closure body may use; the identifier in front of one of
# these is what the closure captures mutably.
MUTATOR_RE = re.compile(r'\b([A-Za-z_]\w*)\.(insert|push|extend|extend_from_slice|remove|clear|set|pop|append)\s*\(')


def function_of(lines, index):
    """Start line of the innermost `fn`/`impl fn` containing `index` (or 0)."""
    for i in range(index, -1, -1):
        if re.match(r'\s*(?:pub\s+)?(?:async\s+)?fn\s+\w+', lines[i]):
            return i
    return 0


def function_end(lines, start):
    """Last line of the function that begins at `start` (file end as a guard)."""
    for i in range(start + 1, len(lines)):
        if lines[i] and not lines[i][0].isspace() and lines[i].startswith('}'):
            return i
    return len(lines) - 1


def closure_spans(lines):
    """(name, start, end, mutated-locals) for every `let … = |…| {…}` closure."""
    out = []
    for i, line in enumerate(lines):
        m = re.match(r'\s*let\s+(?:mut\s+)?([A-Za-z_]\w*)\s*=\s*(?:move\s+)?\|', line)
        if not m:
            continue
        depth = line.count('{') - line.count('}')
        j = i
        while depth > 0 and j + 1 < len(lines):
            j += 1
            depth += lines[j].count('{') - lines[j].count('}')
        body = '\n'.join(lines[i:j + 1])
        mutated = {match[0] for match in MUTATOR_RE.findall(body)}
        out.append((m.group(1), i, j, mutated))
    return out


def check_overlapping_closures(files):
    """Two closures mutating one local, both still live when the other runs."""
    errors = []
    for path in sorted(files):
        lines = strip_code(open(path, encoding='utf-8').read()).split('\n')
        for name_a, start_a, end_a, muts_a in closure_spans(lines):
            for name_b, start_b, end_b, muts_b in closure_spans(lines):
                if name_b == name_a or start_b <= start_a:
                    continue
                if function_of(lines, start_a) != function_of(lines, start_b):
                    continue
                shared = muts_a & muts_b
                if not shared:
                    continue
                # A later call of the first closure after the second was
                # declared means both borrows are live at once.
                call = re.compile(r'\b' + re.escape(name_a) + r'\s*\(')
                fn_end = function_end(lines, function_of(lines, start_a))
                if any(call.search(lines[k]) for k in range(start_b, fn_end + 1)):
                    errors.append(
                        f'{path}:{start_a + 1}: closures `{name_a}` and `{name_b}` both '
                        f'mutate `{sorted(shared)[0]}` while both are live '
                        f'(borrow-checked error: one closure must do both)')
    return errors


def main():
    src_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.getcwd(), 'src')
    root_file = os.path.join(src_dir, 'main.rs')
    root = Mod((), root_file)
    load_module(root, root_file)
    checked, errors = check(root, src_dir)
    errors.extend(check_local_calls(root, src_dir))
    traits, impls, files = collect_traits(src_dir)
    errors.extend(check_traits(traits, impls, src_dir))
    errors.extend(check_struct_literals(files))
    errors.extend(check_enum_variants(files))
    errors.extend(check_split_derives(files))
    errors.extend(check_bare_module_paths(files))
    errors.extend(check_overlapping_closures(files))
    print(f'resolved {checked} crate:: path(s) through {count_mods(root)} module(s)')
    for file, line, name, path in MISSING:
        errors.append(
            f'{file}:{line}: `mod {name};` has no file (expected '
            f'{"/".join(path)}.rs or {"/".join(path)}/mod.rs)')
    for e in errors:
        print('  ' + e)
    print(f'{"FAIL" if errors else "OK"}: {len(errors)} unresolved path/arity error(s)')
    return 1 if errors else 0


def count_mods(mod):
    return 1 + sum(count_mods(c) for c in mod.children.values())


if __name__ == '__main__':
    sys.exit(main())

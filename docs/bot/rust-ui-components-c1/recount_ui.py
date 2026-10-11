#!/usr/bin/env python3
"""C1 source-only inventory. Python stdlib + existing git. No network, builds or writes.

stdout is deterministic JSON. CSS signatures are declaration vectors, NOT computed
styles, DOM component counts or a full CSS/TS parser. See INVENTORY.md for limits.
"""
import argparse
import collections
import hashlib
import json
import re
import subprocess
from pathlib import Path

SOURCE_SHA = 'e1ab622aa7708ae883a3cdd83d042fbf2cf31e38'
EXTRA = ['web/src/app.ts', 'web/src/app-entry.ts', 'web/src/i18n.ts']
STYLE_KEYS = {'color', 'background', 'background-color', 'border', 'border-color',
              'border-radius', 'height', 'min-height', 'width', 'min-width', 'max-width',
              'padding', 'padding-inline', 'padding-block', 'margin', 'gap', 'font',
              'font-size', 'font-weight', 'line-height', 'opacity', 'outline',
              'outline-offset', 'box-shadow', 'display', 'position', 'z-index'}
HEX = re.compile(r'(?<![\w-])#[0-9a-fA-F]{3,8}\b')
RGB = re.compile(r'\brgba?\([^)]*\)', re.I)
VAR = re.compile(r'var\(\s*(--[\w-]+)')

def clean_css(s):
    return re.sub(r'/\*.*?\*/', lambda m: ''.join('\n' if c == '\n' else ' ' for c in m[0]), s, flags=re.S)

def split_top(s, delimiter):
    start = 0; depth = 0; quote = None; escape = False
    for i, c in enumerate(s):
        if escape: escape = False; continue
        if c == '\\': escape = True; continue
        if quote:
            if c == quote: quote = None
        elif c in "\"'": quote = c
        elif c in '([': depth += 1
        elif c in ')]': depth -= 1
        elif c == delimiter and depth == 0:
            yield s[start:i]; start = i + 1
    yield s[start:]

def css_rules(text, warnings=None):
    """Balanced blocks, nested @ contexts; comments preserve source line offsets."""
    s = clean_css(text)
    def walk(start, end, contexts):
        pos = start
        while pos < end:
            # At-rule statements such as @import end before the next block.
            begin = pos; quote = None; parens = 0; escape = False
            while pos < end:
                c = s[pos]
                if escape: escape = False
                elif c == '\\': escape = True
                elif quote:
                    if c == quote: quote = None
                elif c in "\"'": quote = c
                elif c == '(': parens += 1
                elif c == ')': parens -= 1
                elif c == ';' and parens == 0: begin = pos + 1
                elif c == '{' and parens == 0: break
                pos += 1
            if pos == end: return
            header_raw = s[begin:pos]; header = ' '.join(header_raw.split())
            line = s.count('\n', 0, begin + len(header_raw) - len(header_raw.lstrip())) + 1
            body_start = pos + 1; pos += 1; depth = 1; quote = None; escape = False
            while pos < end and depth:
                c = s[pos]
                if escape: escape = False
                elif c == '\\': escape = True
                elif quote:
                    if c == quote: quote = None
                elif c in "\"'": quote = c
                elif c == '{': depth += 1
                elif c == '}': depth -= 1
                pos += 1
            body_end = pos-1 if depth == 0 else pos
            if depth and warnings is not None:
                warnings.append({'line': line, 'unclosed_blocks': depth, 'handling': 'EOF closes block for lexical inventory; browser rendering unverified'})
            body = s[body_start:body_end]
            if header.startswith(('@media', '@supports', '@layer', '@container', '@keyframes', '@-webkit-keyframes')):
                yield from walk(body_start, body_end, contexts + [header])
            else:
                declarations = []
                for item in split_top(body, ';'):
                    if ':' in item:
                        k, v = item.split(':', 1)
                        if re.fullmatch(r'(?:--)?[-\w]+', k.strip()):
                            declarations.append([k.strip().lower(), ' '.join(v.split())])
                yield {'selector': header, 'line': line, 'contexts': contexts, 'declarations': declarations}
    yield from walk(0, len(s), [])

def selected(path):
    p = Path(path)
    return path in ['web/src/index.html', 'web/src/styles.css'] + EXTRA or (
        p.parent.as_posix() == 'web/src/styles' and p.suffix == '.css') or (
        p.parent.as_posix() == 'web/src/app' and p.suffix == '.ts') or (
        p.parent.as_posix() == 'web/src/i18n' and p.suffix == '.json')

def aggregate(values):
    c = collections.Counter(values)
    return {'occurrences': sum(c.values()), 'unique': len(c), 'values': dict(sorted(c.items()))}

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--repo', type=Path, default=Path('.'))
    ap.add_argument('--ref', default=SOURCE_SHA)
    ap.add_argument('--self-test', action='store_true')
    args = ap.parse_args()
    if args.self_test:
        fixture = '/*\nx*/ @import url("x.css"); @media (x) { .a:hover, .b:is(.x,.y) { color: var(--a, rgba(0, 0, 0, .2)); content: "a;b}"; padding: 2px; } }'
        rules = list(css_rules(fixture))
        assert len(rules) == 1 and rules[0]['line'] == 2
        assert rules[0]['contexts'] == ['@media (x)']
        assert rules[0]['declarations'][1] == ['content', '"a;b}"']
        assert len(list(split_top(rules[0]['selector'], ','))) == 2
        assert selected('web/src/app/x.ts') and not selected('web/src/vendor/x.js')
        assert len(list(css_rules('@keyframes x {0%,100% {opacity:0}50%{opacity:1}}'))) == 2
        print('6 self-checks passed; 0 failed; 0 skipped'); return
    def git(*cmd):
        return subprocess.check_output(['git', '-C', str(args.repo), *cmd])
    actual = git('rev-parse', args.ref + '^{commit}').decode().strip()
    if actual != SOURCE_SHA: raise SystemExit('ref is not the C1 fixed source SHA')
    paths = sorted(p for p in git('ls-tree', '-r', '--name-only', actual).decode().splitlines() if selected(p))
    warnings = []; files = []; all_rules = []; lex = {'hex': [], 'rgb': [], 'var': []}; literals = {'hex': [], 'rgb': [], 'var': [], 'font_size': []}
    icon_svg = []; icon_img = []; inline = []; glyphs = []; states = collections.Counter()
    for path in paths:
        data = git('show', actual + ':' + path); text = data.decode('utf-8')
        files.append({'path': path, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(), 'fixture': path.endswith('-fixtures.ts')})
        for kind, rx in [('hex', HEX), ('rgb', RGB), ('var', VAR)]:
            lex[kind].extend(m.group(1) if kind == 'var' else m[0].lower().replace(' ', '') for m in rx.finditer(text))
        if path.endswith('.css'):
            local_warnings = []
            parsed_rules = list(css_rules(text, local_warnings))
            warnings.extend({'path': path, **w} for w in local_warnings)
            for rule in parsed_rules:
                rule['path'] = path; all_rules.append(rule)
                for key, value in rule['declarations']:
                    for kind, rx in [('hex', HEX), ('rgb', RGB), ('var', VAR)]:
                        literals[kind].extend(m.group(1) if kind == 'var' else m[0].lower().replace(' ', '') for m in rx.finditer(value))
                    if key == 'font-size': literals['font_size'].append(value)
                for state in ['hover', 'active', 'disabled', 'focus-visible', 'focus', 'checked']:
                    if re.search(r':' + state + r'(?![-\w])', rule['selector']): states[state] += 1
        elif not path.endswith('-fixtures.ts') and not path.endswith('.json'):
            for label, rx, out in [('svg', r'<svg\b.*?</svg>', icon_svg), ('img', r'<img\b[^>]*>', icon_img), ('inline', r'(?:style\s*=\s*["\']|\.style(?:\.|\[|\s*,)|\.setProperty\()', inline)]:
                for m in re.finditer(rx, text, re.S | re.I):
                    out.append({'path': path, 'line': text.count('\n', 0, m.start())+1, 'text': ' '.join(m[0].split())})
            for m in re.finditer(r'[\u2190-\u2bff\U0001f300-\U0001faff]', text):
                glyphs.append(m[0])
    profiles = {}
    for name, rx in [('buttons', r'(?i)(?:button|btn|chip|pill|close)'), ('overlays', r'(?i)(?:dialog|modal|overlay|backdrop|popover|sheet|drawer)'), ('inputs', r'(?i)(?:input|textarea|select)')]:
        rows = []
        for r in all_rules:
            if any('keyframes' in c for c in r['contexts']): continue
            if not re.search(rx, r['selector']): continue
            vec = [d for d in r['declarations'] if d[0] in STYLE_KEYS]
            if vec:
                rows.append({'path':r['path'],'line':r['line'],'selector':r['selector'],'contexts':r['contexts'],'vector':vec})
        profiles[name] = {'rules': len(rows), 'unique_declaration_vectors': len({json.dumps(r['vector']) for r in rows}), 'rows': rows}
    result = {'source_sha': actual, 'files':files, 'file_count':len(files), 'fixture_count':sum(f['fixture'] for f in files),
              'css_parse_warnings': warnings, 'css_rule_count':len(all_rules), 'css_declaration_values':{k:aggregate(v) for k,v in literals.items()},
              'raw_source_lexical_values_including_comments_fixtures':{k:aggregate(v) for k,v in lex.items()},
              'state_css_rules':dict(states), 'declaration_profiles_not_computed_components':profiles,
              'svg_literal_candidates':icon_svg,'img_literal_candidates':icon_img,'inline_style_candidates':inline,
              'symbol_codepoints_including_comments':aggregate(glyphs),
              'css_conditions':sorted({c for r in all_rules for c in r['contexts']}),
              'custom_property_definitions':[{'path':r['path'],'line':r['line'],'selector':r['selector'],'contexts':r['contexts'],'property':k,'value':v} for r in all_rules for k,v in r['declarations'] if k.startswith('--')]}
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))

if __name__ == '__main__': main()

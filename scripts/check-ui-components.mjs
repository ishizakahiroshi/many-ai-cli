#!/usr/bin/env node
// UI appearance belongs to ui-components.css; inherited exceptions have exact,
// reviewed fingerprints and occurrence limits. A new selector/value cannot hide
// behind a whole-file exemption. See docs/ui-components.md.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const APPEARANCE = /^(?:color|background(?:-color|-image)?|border(?:-[a-z-]+)?|box-shadow|font(?:-size|-family|-weight)?|text-shadow|outline(?:-[a-z-]+)?|fill|stroke)$/;
const LITERAL = /#[\da-f]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\s*\(/i;
const COLOR_NAMES = 'aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgrey darkgreen darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray grey green greenyellow honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgrey lightgreen lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen';
const NAMED = new RegExp(`\\b(?:${COLOR_NAMES.replaceAll(' ', '|')})\\b`, 'i');
const CONTROL = /(?:\bbutton\b|\binput\b|\bselect\b|\btextarea\b|(?:^|[\s.#_-])(?:btn|button|dialog|modal)(?:[\s.#:_-]|$))/i;
const normalize = value => value.trim().replace(/\s+/g, ' ');
const digest = fields => createHash('sha256').update(JSON.stringify(fields)).digest('hex');
const require = createRequire(import.meta.url);
let compiler;
let cssCompiler;

// Parse only: no bundle, build artifact or product process. The pinned frontend
// esbuild parser catches invalid selectors/preludes that a brace lexer cannot.
export function cssSyntaxIssues(source, filename) {
  cssCompiler ||= require(path.join(ROOT, 'web/node_modules/esbuild'));
  const describe = item => `${filename}:${item.location?.line || 1}: ${item.text}`;
  try {
    return cssCompiler.transformSync(source, { loader: 'css', logLevel: 'silent', sourcefile: filename }).warnings.map(describe);
  } catch (error) {
    return (error.errors || [{ text: error.message }]).map(describe);
  }
}

// Quotes, escapes, comments and parentheses are tracked so strings, URLs and
// nested @media/@supports do not become declarations or brace mismatches.
export function cssDeclarations(text, filename = '') {
  const declarations = [], errors = [], stack = [];
  let start = 0, quote = '', comment = false, parens = 0, brackets = 0;
  let segment = '';
  const flush = end => {
    const item = normalize(segment);
    const colon = item.indexOf(':');
    const selector = stack.at(-1);
    if (selector && !selector.startsWith('@') && colon > 0) {
      const property = item.slice(0, colon).trim().toLowerCase();
      if (/^(?:--[\w-]+|[a-z-]+)$/.test(property)) {
        declarations.push({ file: filename, selector, context: stack.slice(0, -1).join(' > '), property,
          value: normalize(item.slice(colon + 1)), line: text.slice(0, start).split('\n').length });
      }
    }
    start = end + 1; segment = '';
  };
  for (let i = 0; i < text.length; i++) {
    const ch = text[i], next = text[i + 1];
    if (comment) { if (ch === '*' && next === '/') { comment = false; i++; segment += ' '; } continue; }
    if (quote) { segment += ch; if (ch === '\\') { segment += text[++i] || ''; } else if (ch === quote) quote = ''; continue; }
    if (ch === '/' && next === '*') { comment = true; i++; continue; }
    if (ch === '"' || ch === "'") { quote = ch; segment += ch; continue; }
    if (ch === '(') parens++;
    if (ch === ')') { if (!parens) errors.push(`${filename}: unexpected closing parenthesis`); else parens--; }
    if (ch === '[') brackets++;
    if (ch === ']') { if (!brackets) errors.push(`${filename}: unexpected closing bracket`); else brackets--; }
    if (!parens && !brackets && ch === '{') { stack.push(normalize(segment)); segment = ''; start = i + 1; }
    else if (!parens && !brackets && ch === '}') { flush(i); if (!stack.length) errors.push(`${filename}:${text.slice(0, i).split('\n').length}: unexpected closing brace`); else stack.pop(); }
    else if (!parens && !brackets && ch === ';') flush(i);
    else segment += ch;
  }
  if (comment || quote || parens || brackets || stack.length) errors.push(`${filename}: unclosed CSS construct (${stack.join(' > ')})`);
  return { declarations, errors };
}

export function appearanceFindings(declarations) {
  return declarations.flatMap(d => {
    if (d.property.startsWith('--')) {
      // Canonical theme/component/footer palettes may define literals. A screen
      // overriding a component token with a raw color is still local appearance.
      const canonical = d.file === 'web/src/styles/ui-components.css'
        || (d.file === 'web/src/styles.css' && /^:root$|^\[data-theme=["']light["']\]$/.test(d.selector))
        || (d.file === 'web/src/styles/token-statusbar.css' && d.property.startsWith('--ui-footer-'));
      if (canonical || !(LITERAL.test(d.value) || NAMED.test(d.value.replace(/var\(--[\w-]+\)/g, '')))) return [];
      return [{ ...d, rule: 'literal-local-token-color', fingerprint: digest([d.file, d.context, d.selector, d.property, d.value, 'literal-local-token-color']) }];
    }
    if (d.file === 'web/src/styles/ui-components.css') return [];
    const rules = [];
    const value = d.value.replace(/url\([^)]*\)/gi, '').replace(/(['"])(?:\\.|(?!\1).)*\1/g, '');
    if (APPEARANCE.test(d.property) && (LITERAL.test(value) || (d.property !== 'font-family' && NAMED.test(value.replace(/var\(--[\w-]+\)/g, ''))))) rules.push('literal-color');
    if ((d.property === 'font-size' || d.property === 'font') && /(?:^|[^\w-])(?:\d*\.?\d+(?:px|rem|em|pt|%)|xx-small|x-small|small|medium|large|x-large|xx-large)(?![\w-])/i.test(value)) rules.push('literal-font-size');
    if (CONTROL.test(d.selector) && APPEARANCE.test(d.property)) rules.push('local-control-appearance');
    return rules.map(rule => ({ ...d, rule, fingerprint: digest([d.file, d.context, d.selector, d.property, d.value, rule]) }));
  });
}

function filesBelow(dir, suffixes) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap(e => {
    const full = path.join(dir, e.name);
    return e.isDirectory() ? filesBelow(full, suffixes) : e.isFile() && suffixes.some(s => e.name.endsWith(s)) ? [full] : [];
  });
}

export function codeFindings(source, filename) {
  compiler ||= require(path.join(ROOT, 'web/node_modules/typescript'));
  const ts = compiler;
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true);
  const findings = [];
  const add = (node, rule, value) => {
    const item = { file: filename, selector: 'Hub confirmation/style call', context: '', property: rule, value: normalize(value),
      line: file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1, rule };
    item.fingerprint = digest([item.file, item.context, item.selector, item.property, item.value, rule]);
    findings.push(item);
  };
  const visit = node => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'appConfirm') {
      const options = node.arguments[0];
      const action = options && ts.isObjectLiteralExpression(options) && options.properties.some(p =>
        ts.isPropertyAssignment(p) && (ts.isIdentifier(p.name) || ts.isStringLiteral(p.name)) && p.name.text === 'action');
      if (!action) add(node, 'confirmation-without-action', node.getText(file));
    }
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && ts.isPropertyAccessExpression(node.left) &&
      ts.isPropertyAccessExpression(node.left.expression) && node.left.expression.name.text === 'style' &&
      (ts.isStringLiteral(node.right) || ts.isNoSubstitutionTemplateLiteral(node.right))) {
      const property = node.left.name.text.replace(/[A-Z]/g, c => '-' + c.toLowerCase());
      const declarations = property === 'css-text'
        ? cssDeclarations(`.inline { ${node.right.text} }`, filename).declarations
        : [{ file: filename, context: '', selector: '.inline', property, value: node.right.text, line: 0 }];
      const parsed = appearanceFindings(declarations);
      if (parsed.length) add(node, 'literal-inline-appearance', node.getText(file));
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return findings;
}

export function audit(root = ROOT, exceptions = { entries: [] }) {
  const errors = [], findings = [];
  const cssFiles = filesBelow(path.join(root, 'web/src'), ['.css']).filter(f => !f.includes(`${path.sep}vendor${path.sep}`));
  for (const file of cssFiles) {
    const rel = path.relative(root, file).replaceAll(path.sep, '/');
    const source = fs.readFileSync(file, 'utf8');
    const parsed = cssDeclarations(source, rel);
    errors.push(...cssSyntaxIssues(source, rel));
    errors.push(...parsed.errors); findings.push(...appearanceFindings(parsed.declarations));
  }
  const codeFiles = filesBelow(path.join(root, 'web/src'), ['.ts']).filter(f => !/-fixtures\.ts$/.test(f) && !f.includes(`${path.sep}vendor${path.sep}`));
  const index = path.join(root, 'web/src/index.html');
  if (fs.existsSync(index)) codeFiles.push(index);
  // Literal HTML style attributes, including those inside TS templates. Dynamic
  // .style assignments are reviewed at their caller; this is not a JS dataflow audit.
  for (const file of codeFiles) {
    const rel = path.relative(root, file).replaceAll(path.sep, '/');
    const source = fs.readFileSync(file, 'utf8');
    if (file.endsWith('.ts')) findings.push(...codeFindings(source, rel));
    for (const match of source.matchAll(/\bstyle\s*=\s*(["'])(.*?)\1/gs)) {
      const parsed = cssDeclarations(`.inline { ${match[2]} }`, rel);
      findings.push(...appearanceFindings(parsed.declarations).filter(f => f.rule !== 'local-control-appearance').map(f => ({
        ...f, line: source.slice(0, match.index).split('\n').length + f.line - 1,
      })));
    }
  }
  const allowances = new Map();
  for (const e of exceptions.entries || []) {
    if (!e.reason?.trim() || !/^[a-f0-9]{64}$/.test(e.fingerprint || '') || !Number.isInteger(e.count) || e.count < 1 || allowances.has(e.fingerprint)) {
      errors.push('Invalid or duplicate reviewed UI exception'); continue;
    }
    allowances.set(e.fingerprint, e.count);
  }
  const used = new Map(), blocked = [];
  for (const f of findings) {
    const count = (used.get(f.fingerprint) || 0) + 1; used.set(f.fingerprint, count);
    if (count > (allowances.get(f.fingerprint) || 0)) blocked.push(f);
  }
  // Removal is safe; report obsolete exceptions for cleanup without blocking it.
  const stale = [...allowances.keys()].filter(key => !used.has(key));
  return { cssFiles: cssFiles.length, codeFiles: codeFiles.length, errors, blocked, findings, staleExceptions: stale.length };
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url))) {
  const args = process.argv.slice(2);
  const rootFlag = args.indexOf('--root');
  const root = rootFlag >= 0 ? path.resolve(args[rootFlag + 1]) : ROOT;
  const exceptionsFlag = args.indexOf('--exceptions');
  const exceptionPath = exceptionsFlag >= 0 ? path.resolve(args[exceptionsFlag + 1]) : path.join(root, 'scripts/ui-components-exceptions.json');
  const exceptions = JSON.parse(fs.readFileSync(exceptionPath, 'utf8'));
  const result = audit(root, exceptions);
  if (args.includes('--json')) console.log(JSON.stringify(result));
  else {
    for (const error of result.errors) console.error(error);
    for (const item of result.blocked.slice(0, 25)) console.error(`${item.file}:${item.line} ${item.rule}: ${item.selector} { ${item.property}: ${item.value} }`);
    console.log(`UI components: ${result.cssFiles} CSS / ${result.codeFiles} code files, ${result.errors.length} syntax/config errors, ${result.blocked.length} new appearance declarations; ${result.staleExceptions} stale exceptions.`);
  }
  if (result.errors.length || result.blocked.length) process.exitCode = 1;
}

/*
 * Ratchet sensor for the Aurora Glass design system (Slice 5).
 *
 * The redesign repeatedly risked two regressions that a browser screenshot
 * won't reliably catch: (1) a console glass rule leaking out of the
 * Work/Status/Settings scope into Home/legacy surfaces, and (2) the
 * motion / backdrop-filter robustness invariants silently dropping. This static
 * source guard fails the suite if any of those invariants regress.
 *
 * Static (parses CSS source) rather than computed-style (needs a browser): it
 * runs in the existing node:test harness with no new dependency, and catches
 * the leakage class of bug Codex flagged during Slices 3-4.
 *
 * SCOPE & LIMITS: `isScoped` is a hardened heuristic, not a CSS-spec tokenizer.
 * Through adversarial review it was closed against the realistic leak shapes —
 * comma arms, comments (incl. inside quoted attributes), :not(), :is()/:where()
 * with non-scope alternatives, every combinator (descendant/child/sibling),
 * pseudo/attribute root qualifiers, quoted brackets/escapes — and it fails
 * CLOSED on malformed input and DENIES by default (any unscoped `.vnext-*` arm
 * is an offender). Its job is to catch a developer accidentally adding an
 * unscoped console rule in hand-authored CSS, which it does robustly; it is not
 * a security boundary against deliberately pathological selectors. If real
 * console.css ever needs a selector this heuristic can't classify, prefer a
 * proper CSS selector parser over widening the heuristic.
 */
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', 'src', 'vnext', 'design');
const read = (rel) => readFileSync(join(root, rel), 'utf8');

// String-aware comment stripping: a `/* */` inside a quoted attribute value is
// literal string content in CSS, not a comment, so a naive regex could erase a
// real selector and hide a sibling escape. Scan char-by-char, only treating
// `/* */` as a comment when outside a string.
function stripComments(css) {
  let out = '';
  let i = 0;
  let quote = null;
  while (i < css.length) {
    const ch = css[i];
    if (quote) {
      if (ch === '\\') { out += css.slice(i, i + 2); i += 2; continue; }
      if (ch === quote) quote = null;
      out += ch;
      i += 1;
      continue;
    }
    if (ch === '"' || ch === "'") { quote = ch; out += ch; i += 1; continue; }
    if (ch === '/' && css[i + 1] === '*') {
      i += 2;
      while (i < css.length && !(css[i] === '*' && css[i + 1] === '/')) i += 1;
      i += 2;
      continue;
    }
    out += ch;
    i += 1;
  }
  return out;
}
// The console surface scopes to Work/Status/Settings. Each surface declares
// which view-root class(es) count as "scoped" so the SAME heuristic guards
// every file without one widening the other. `single` captures the bare-class
// shape, `cls` the full-string shape for :is()/:where() alternatives. Default
// = console scope (preserves the original behavior).
const CONSOLE_SCOPE = {
  single: /^\.vnext-app-(work|status|settings)/,
  cls: /^\.vnext-app-(work|status|settings)$/,
};
const SCOPE_CLASS = CONSOLE_SCOPE.cls;

// An arm is scoped iff its LEADING compound is a view-root scope AND it enters
// that root's subtree (descendant ` ` or child `>`), never a sibling (`+`/`~`,
// which selects OUTSIDE the root). The leading scope is either a bare scope
// class or an :is()/:where() whose EVERY alternative is a scope class — so
// `:is(scope, *)` / `:where(scope, body)` fail (a non-scope alternative widens
// back to global), while a 2-of-3 subset like `:is(.vnext-app-status,
// .vnext-app-settings)` passes. Positive enumeration: we require the shapes the
// codebase actually uses rather than trying to prove arbitrary selectors safe.
// `scope` selects which view-root class(es) count as scoped (default: console).
function isScoped(arm, scope = CONSOLE_SCOPE) {
  const a = stripComments(arm).trim();
  // 1. Match the root TOKEN (bare scope class, or :is/:where of only scope classes).
  let rootLen = -1;
  const single = a.match(scope.single);
  if (single && /^($|[\s.:#[>~+])/.test(a.slice(single[0].length))) {
    rootLen = single[0].length;
  } else {
    const isw = a.match(/^:(?:is|where)\(([^)]*)\)/);
    if (isw) {
      const alts = isw[1].split(',').map((s) => s.trim()).filter(Boolean);
      if (alts.length > 0 && alts.every((s) => scope.cls.test(s))) rootLen = isw[0].length;
    }
  }
  if (rootLen < 0) return false;
  // 2. Consume the REST of the leading compound (pseudo-classes, attributes,
  //    compound classes — e.g. `:hover`, `[data-x]`), tracking bracket depth,
  //    until the first top-level whitespace or combinator.
  let i = rootLen;
  let depth = 0;
  let quote = null;
  while (i < a.length) {
    const ch = a[i];
    if (quote) {
      if (ch === '\\') { i += 2; continue; } // skip escaped char
      if (ch === quote) quote = null;
      i += 1;
      continue;
    }
    if (ch === '"' || ch === "'") { quote = ch; i += 1; continue; }
    if (ch === '(' || ch === '[') depth += 1;
    else if (ch === ')' || ch === ']') depth -= 1;
    else if (depth === 0 && /[\s>+~]/.test(ch)) break;
    i += 1;
  }
  // Fail closed on a malformed compound (unterminated string / unbalanced
  // brackets) — a parser we can't trust must not be treated as scoped.
  if (quote || depth !== 0) return false;
  // 3. The first combinator after the root compound decides: a sibling
  //    combinator (`+`/`~`) selects OUTSIDE the root and is rejected; descendant
  //    or child (`>`) stays inside the subtree and is accepted.
  const rest = a.slice(i).replace(/^\s+/, '');
  return rest === '' || (rest[0] !== '+' && rest[0] !== '~');
}

// A keyframe step head is ONLY `from`, `to`, or percentage(s) — never a real
// selector. (Skipping any head containing `%` was too broad: it would drop a
// legitimate selector like `.vnext-progress[aria-valuetext="50%"]`.)
function isKeyframeStep(head) {
  const arms = topLevelArms(head);
  return arms.length > 0 && arms.every((a) => /^(from|to|-?\d+(\.\d+)?%)$/.test(a.trim()));
}

function selectorBlocks(css) {
  // Split into "selector { ... }" heads; ignore at-rules and keyframe steps.
  const blocks = [];
  const re = /([^{}]+)\{/g;
  let m;
  while ((m = re.exec(stripComments(css))) !== null) {
    const sel = m[1].trim();
    if (!sel || sel.startsWith('@') || isKeyframeStep(sel)) continue;
    blocks.push(sel);
  }
  return blocks;
}

// Split a selector head into its top-level comma arms, ignoring commas nested
// inside parentheses (e.g. inside :is(...)). This is what makes the guard
// closed: each arm is checked on its own, so a mixed "scoped, unscoped" list
// cannot pass on the strength of the scoped arm alone.
function topLevelArms(selector) {
  const arms = [];
  let depth = 0;
  let cur = '';
  for (const ch of selector) {
    if (ch === '(') depth += 1;
    else if (ch === ')') depth -= 1;
    if (ch === ',' && depth === 0) {
      arms.push(cur.trim());
      cur = '';
    } else {
      cur += ch;
    }
  }
  if (cur.trim()) arms.push(cur.trim());
  return arms;
}

// Default-deny: ANY arm that references a .vnext-* class must be scoped to a
// view root. (A narrow allowlist of component classes would silently miss
// shared primitives like .vnext-hero / .vnext-detail that also appear in
// other views.) Arms that are purely the scope root (e.g. the palette
// rule) reference only .vnext-app-* and pass via isScoped.
function unscopedComponentArms(css, scope = CONSOLE_SCOPE) {
  const offenders = [];
  for (const head of selectorBlocks(css)) {
    for (const arm of topLevelArms(head)) {
      if (/\.vnext-/.test(arm) && !isScoped(arm, scope)) offenders.push(arm);
    }
  }
  return offenders;
}

describe('scope-guard logic (self-test, must be closed not fail-open)', () => {
  it('flags an unscoped arm hidden in a mixed comma selector', () => {
    const leaky = ':is(.vnext-app-work, .vnext-app-status) .vnext-panel, .vnext-panel { color: red; }';
    assert.deepEqual(unscopedComponentArms(leaky), ['.vnext-panel']);
  });
  it('passes a fully-scoped comma selector', () => {
    const ok = ':is(.vnext-app-work, .vnext-app-status) .vnext-panel, .vnext-app-settings .vnext-panel { color: red; }';
    assert.deepEqual(unscopedComponentArms(ok), []);
  });
  it('does not split commas nested inside :is()', () => {
    assert.deepEqual(
      topLevelArms(':is(.vnext-app-work, .vnext-app-status, .vnext-app-settings) .vnext-panel'),
      [':is(.vnext-app-work, .vnext-app-status, .vnext-app-settings) .vnext-panel'],
    );
  });
  it('rejects a scope token that only appears inside a CSS comment', () => {
    const leaky = '/* .vnext-app-work */ .vnext-panel { color: red; }';
    assert.deepEqual(unscopedComponentArms(leaky), ['.vnext-panel']);
  });
  it('rejects a scope token used only inside :not() (negation is not scoping)', () => {
    const leaky = ':not(.vnext-app-work) .vnext-panel { color: red; }';
    assert.deepEqual(unscopedComponentArms(leaky), [':not(.vnext-app-work) .vnext-panel']);
  });
  it('rejects :is(scope, *) — the * alternative makes it global', () => {
    const leaky = ':is(.vnext-app-work, *) .vnext-panel { color: red; }';
    assert.deepEqual(unscopedComponentArms(leaky), [':is(.vnext-app-work, *) .vnext-panel']);
  });
  it('rejects :where(scope, body) — the body alternative makes it global', () => {
    const leaky = ':where(.vnext-app-status, body) .vnext-panel { color: red; }';
    assert.deepEqual(unscopedComponentArms(leaky), [':where(.vnext-app-status, body) .vnext-panel']);
  });
  it('rejects a sibling combinator after the root (selects outside the subtree)', () => {
    assert.deepEqual(unscopedComponentArms('.vnext-app-work + .vnext-panel { color: red; }'), ['.vnext-app-work + .vnext-panel']);
    assert.deepEqual(unscopedComponentArms(':is(.vnext-app-work, .vnext-app-status) ~ .vnext-panel { x: 1; }'), [':is(.vnext-app-work, .vnext-app-status) ~ .vnext-panel']);
  });
  it('accepts a child combinator (still inside the root subtree)', () => {
    assert.deepEqual(unscopedComponentArms('.vnext-app-work > .vnext-panel { color: red; }'), []);
  });
  it('accepts a sibling combinator deeper in the selector (both inside the root)', () => {
    assert.deepEqual(unscopedComponentArms('.vnext-app-work .vnext-x ~ .vnext-panel { color: red; }'), []);
  });
  it('rejects a sibling escape hidden behind a root pseudo/attribute qualifier', () => {
    assert.deepEqual(unscopedComponentArms('.vnext-app-work:hover + .vnext-panel { x: 1; }'), ['.vnext-app-work:hover + .vnext-panel']);
    assert.deepEqual(unscopedComponentArms(':is(.vnext-app-work, .vnext-app-status):focus-within ~ .vnext-panel { x: 1; }'), [':is(.vnext-app-work, .vnext-app-status):focus-within ~ .vnext-panel']);
  });
  it('still accepts a root state qualifier that stays inside the subtree', () => {
    assert.deepEqual(unscopedComponentArms('.vnext-app-work:hover .vnext-panel { x: 1; }'), []);
  });
  it('rejects a sibling escape hidden behind a quoted attribute bracket', () => {
    assert.deepEqual(
      unscopedComponentArms('.vnext-app-work[data-x="[foo"] + .vnext-panel { x: 1; }'),
      ['.vnext-app-work[data-x="[foo"] + .vnext-panel'],
    );
    assert.deepEqual(
      unscopedComponentArms('.vnext-app-work[data-x="]bar"] ~ .vnext-panel { x: 1; }'),
      ['.vnext-app-work[data-x="]bar"] ~ .vnext-panel'],
    );
  });
  it('accepts a quoted attribute value that stays inside the subtree', () => {
    assert.deepEqual(unscopedComponentArms('.vnext-app-work[data-x="a b"] .vnext-panel { x: 1; }'), []);
  });
  it('rejects comment delimiters hidden inside quoted attribute values', () => {
    const leaky = '.vnext-app-work[data-x="/*"] + .vnext-panel[data-y="*/"] { color: red; }';
    assert.deepEqual(unscopedComponentArms(leaky), [leaky.replace(' { color: red; }', '')]);
  });
  it('does not skip a real selector that merely contains a percent in an attribute', () => {
    const leaky = '.vnext-progress[aria-valuetext="50%"] { color: red; }';
    assert.deepEqual(unscopedComponentArms(leaky), ['.vnext-progress[aria-valuetext="50%"]']);
  });
  it('still skips genuine keyframe step heads', () => {
    assert.equal(isKeyframeStep('from'), true);
    assert.equal(isKeyframeStep('0%, 100%'), true);
    assert.equal(isKeyframeStep('.vnext-progress[aria-valuetext="50%"]'), false);
  });
});

describe('balancedBlock helper (used by the materials/motion invariants)', () => {
  it('captures only the at-rule block, not content after it', () => {
    const css = '@media (prefers-reduced-motion: reduce){ a{ x:1 } }\n.vnext-app::after{ y:2 }';
    const block = balancedBlock(css, /@media \(prefers-reduced-motion: reduce\)/);
    assert.ok(block.includes('x:1'));
    assert.ok(!block.includes('y:2'), 'must not leak content after the balanced block');
  });
  it('is not fooled by braces inside comments or strings', () => {
    const css = [
      '@media (prefers-reduced-motion: reduce){',
      '  /* a stray { brace in a comment */',
      '  a{ content: "}" }',          // brace inside a string
      '}',
      '.vnext-app::after{ animation: x }', // post-block invariant text
      '/* } */',                            // stray close brace in a comment
    ].join('\n');
    const block = balancedBlock(css, /@media \(prefers-reduced-motion: reduce\)/);
    assert.ok(block.includes('content: "}"'), 'must include the real block body');
    assert.ok(!block.includes('.vnext-app::after'), 'must not leak post-block CSS via comment/string braces');
  });
  it('does not match an at-rule head that only appears in a comment', () => {
    const css = '/* @media (prefers-reduced-motion: reduce) { } */\n.unrelated { animation-delay: 0ms !important }';
    const block = balancedBlock(css, /@media \(prefers-reduced-motion: reduce\)/);
    assert.equal(block, null, 'a commented-out head must not select the next unrelated block');
  });
});

describe('console.css scope containment', () => {
  const css = read('surfaces/console.css');

  it('never uses the leaky blanket :not(.vnext-app-chat-home) selector', () => {
    assert.doesNotMatch(css, /\.vnext-app:not\(/,
      'console.css must scope to vnext-app-{work,status,settings}, not exclude chat-home');
  });

  it('scopes every styled console component arm to a target view root', () => {
    const offenders = unscopedComponentArms(css);
    assert.deepEqual(offenders, [],
      `unscoped console selector arm(s) would leak into other views: ${JSON.stringify(offenders)}`);
  });

  it('does not accept the chat-home root as a console scope (scopes do not cross-widen)', () => {
    // A bare `.vnext-app-chat-home .vnext-panel` arm is OUT of the console
    // scope, so the console guard must still flag it — proving another
    // surface's root never widens CONSOLE_SCOPE.
    assert.deepEqual(unscopedComponentArms('.vnext-app-chat-home .vnext-panel { x: 1; }'),
      ['.vnext-app-chat-home .vnext-panel']);
  });
});

// SP1.6: the action buttons moved from the styles.css `.vnext-*-button` family
// to the shared VNextButton (`.vnext-btn`), styled in design/surfaces/button.css.
// `.vnext-btn` is GLOBAL by design (it renders in every view-root, not scoped to
// a .vnext-app-* root), so a "restate under a view root" remedy doesn't apply.
// The invariant it must instead uphold is stronger and file-appropriate: the
// shared button surface must be FULLY TOKENIZED — zero literal background/border
// affordance — so it is adaptive by construction in every theme. Without this, a
// future hardcoded fill/border on a `.vnext-btn` variant would reproduce the
// light-mode invisible-affordance bug, yet pass unseen. (Codex flagged this on
// the SP1.6 review.)
describe('shared VNextButton surface is fully tokenized (no literal affordance)', () => {
  const buttonCss = stripComments(read('surfaces/button.css'));
  // Same affordance props + literal detector the root ratchet uses: only the
  // fill/border that DEFINE the light/dark affordance — box-shadow highlights and
  // text color are intentionally excluded (they don't strand the control), and
  // var()-backed values are stripped first so a token never looks literal.
  const AFFORDANCE = /^(background|background-color|border|border-color|border-(top|right|bottom|left)(-color)?)$/;
  const LITERAL = /rgba?\(\s*[\d.]|#[0-9a-fA-F]{3,6}/;
  const rules = [...buttonCss.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter((m) => /\.vnext-btn/.test(m[1]));
  const offenders = [];
  for (const m of rules) {
    for (const decl of m[2].split(';')) {
      const ci = decl.indexOf(':');
      if (ci < 0) continue;
      const prop = decl.slice(0, ci).trim();
      if (!AFFORDANCE.test(prop)) continue;
      if (LITERAL.test(decl.slice(ci + 1).replace(/var\([^)]*\)/g, '')))
        offenders.push(`${m[1].trim()} { ${prop}:${decl.slice(ci + 1).trim()} }`);
    }
  }

  it('sensor is live — actually scanned the .vnext-btn variant rules', () => {
    // Guard against the file moving/renaming and the scan silently going dark.
    const variants = rules.filter((m) =>
      /\.vnext-btn-(primary|secondary|destructive|cta|ghost)(?![a-z0-9-])/.test(m[1]));
    assert.ok(variants.length >= 5,
      `expected to scan the .vnext-btn variant rules; found ${variants.length}`);
  });

  it('every .vnext-btn background/border resolves through a token (adaptive in both themes)', () => {
    assert.deepEqual(offenders, [],
      `the shared VNextButton surface (design/surfaces/button.css) introduced a LITERAL ` +
      `background/border — it will not adapt to the light theme. Tokenize it via a ` +
      `--btn-* var in theme.css: ` +
      `${JSON.stringify(offenders)}`);
  });
});

// Extract the balanced `{ ... }` block that follows an at-rule head, so an
// assertion can't be satisfied by content that lives AFTER the block. Comments
// are removed FIRST (string-aware) — so neither the head discovery nor the
// brace counter can be fooled by a commented-out at-rule or a `/* { */`. Braces
// inside quoted strings are still skipped during counting.
function balancedBlock(rawCss, headRegex) {
  const css = stripComments(rawCss);
  const idx = css.search(headRegex);
  if (idx < 0) return null;
  const open = css.indexOf('{', idx);
  if (open < 0) return null;
  let depth = 0;
  let quote = null;
  for (let i = open; i < css.length; i += 1) {
    const ch = css[i];
    if (quote) {
      if (ch === '\\') { i += 1; continue; }
      if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'") { quote = ch; continue; }
    if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) return css.slice(open, i + 1);
    }
  }
  return null;
}

describe('materials.css robustness invariants', () => {
  const css = read('materials.css');
  const block = balancedBlock(css, /@supports not[^{]*backdrop-filter/);
  it('has a no-backdrop-filter @supports fallback block', () => {
    assert.ok(block, 'materials.css must have an @supports not(backdrop-filter) fallback');
  });
  it('redefines EVERY glass-fill variable in the fallback (not just one)', () => {
    for (const v of ['--glass-fill:', '--glass-fill-raised:', '--glass-fill-floating:']) {
      assert.ok(block.includes(v), `no-backdrop fallback is missing ${v} — that surface stays translucent without blur`);
    }
  });
});

describe('motion.css reduced-motion invariants', () => {
  const css = read('motion.css');
  const block = balancedBlock(css, /@media \(prefers-reduced-motion: reduce\)/);
  it('has a reduced-motion block', () => assert.ok(block, 'motion.css must gate motion behind reduced-motion'));
  it('zeros animation-delay INSIDE the block (so delayed entrances do not snap in)', () => {
    assert.match(block, /animation-delay:\s*0ms\s*!important/);
  });
  it('covers root-element pseudos INSIDE the block (the chat-home aurora)', () => {
    assert.match(block, /\.vnext-app::(before|after)/);
  });
});

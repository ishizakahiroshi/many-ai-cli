# Pinned Go Unicode regex compatibility

The authorization oracle is `internal/autoapproval/policy.go` at
`21d0bc7935a2c4696fb89ccff2e324157a528c2d`, which calls Go `regexp.Compile` and
boolean `MatchString`. This extension fixes its previously recorded valid-Go
Unicode property, case-folding, and surrogate-pattern compatibility gaps. It
only concerns valid UTF-8 Rust strings, the policy API's existing input domain.

## Provenance and reproducibility

`generate.go` requires **Go 1.26.8 / Unicode 15.0.0**, verifies exact SHA-256
hashes of the compiler's `VERSION`, Go license, Unicode table/folding source,
and regexp parser/group source, and uses only that standard library. The
version, source URLs, and hashes are recorded in `provenance.json`. Source is
available in the official Go tag:
<https://go.googlesource.com/go/+/refs/tags/go1.26.8/>.

The emitted `rust/src/approval/policy/go_regex/tables.rs` contains explicit rune
intervals from `regexp/syntax.Parse` for every accepted category, script, and
special property, separately with and without FoldCase. It also contains all
nontrivial `unicode.SimpleFold` transitions. These data are derived from the
Go Authors' source. The complete BSD-3-Clause license is retained beside the
data as `GO-LICENSE`; source and binary redistributions must preserve its
notices. There is no runtime dependence on Go or Rust's Unicode tables.

Run from the repository root, with `GO_BIN` set to an already installed,
approved Go 1.26.8 executable:

```sh
cache=$(mktemp -d)
GOPROXY=off GOSUMDB=off GOTOOLCHAIN=local GOENV=off GO111MODULE=off \
  GOCACHE="$cache/cache" GOPATH="$cache/modules" GOTMPDIR="$cache" \
  "$GO_BIN" run rust/tests/fixtures/services/autoapproval-unicode/generate.go \
  rust/src/approval/policy/go_regex/tables.rs \
  rust/tests/fixtures/services/autoapproval-unicode
```

The program reads only pinned toolchain source and writes the named generated
outputs. It does not inspect a policy, user home, configuration, provider,
network, or repository history. It neither sets nor discovers HOME. Generate
twice into different output directories and byte-compare all four generated
files to check reproducibility. The generated table uses rustfmt-skip only for
its deterministic data arrays; handwritten Rust follows normal formatting.

## Matching and dialect contract

`approval/policy/go_regex.rs` proposes the integration API:

```rust
pub(super) fn compile(pattern: &str) -> Result<regex::Regex, RegexIssue>
```

The integration owner registers this child module and replaces the old local
translator with this function. The function and enum are restricted to the
policy parent. It needs no new dependency. All property classes and every
case-insensitive literal/range/POSIX/shorthand class become explicit scalar
intervals before reaching `regex`. Case folding is scoped exactly to the
active Go flags, including disabling `i` inside a group. Negated classes are
folded before complementing; a complemented set is not folded a second time.
ASCII word boundaries and Go's ASCII shorthands remain ASCII.

Go permits surrogate rune escapes although a surrogate cannot occur in a
UTF-8 Rust string. Nonsingleton rune-set union, folding and complement operate
on the full Go rune domain; emission then intersects with Unicode scalar
values. A surrogate-only nonsingleton range is the empty language, preserving
its `?`, `*`, `{0}`, alternation, and complement semantics.

Singleton surrogates have a pinned-Go optimizer exception. In
`onepass.go:onePassPrefix`, `strings.Builder.WriteRune` encodes a surrogate
literal as U+FFFD; `exec.go:doOnePass` subsequently skips that instruction.
Thus `^\x{d800}$` matches U+FFFD, while `^\x{d800}?$` does not. The adapter
reproduces the source-provable straight BeginText/literal/optional-EndText
case, including noncapturing literal groups, singleton classes, and the
prefix loop's stop before an actual U+FFFD rune. It does not apply this
transformation to unanchored expressions, for which Go cannot select the
one-pass executor. An explicit `Unsupported` warning retains the remaining
gap for more general BeginText candidates containing a singleton surrogate,
such as repetitions, captures, mixed classes, or empty-group Nops. There are
29 individually enumerated valid-Go gap cases in this corpus. They assert the
precise unsupported diagnosis instead of silently accepting narrower matching
or broadly substituting U+FFFD. A full Go optimizer port is out of this slice.

One pinned-source detail matters: the 1.26.8 parser normalizes property names
before its direct lookup in `unicode.Scripts`. Some noncanonical script-map
keys (for example `Old_Italic`) are consequently rejected even though the raw
Unicode map contains them. Only compiler-accepted names are emitted; rejected
map names are retained in the oracle corpus. Category aliases use the
parser's separate canonical alias map. Broadly accepting all raw script names
would change the pinned dialect.

The adapter also handles Go-specific repeat resets after flag directives or
empty quoted spans, unbounded/count-zero nested repetition, names on captures,
quoted literals, octal/hex escapes, literal braces, and unterminated POSIX
class-prefix treatment. Match capture values and greedy match selection are
outside the policy's boolean-matching interface. Greediness cannot change the
existence of a match; `U` and lazy quantifiers are validated but need not be
forwarded to the backend.

## Evidence and remaining limits

The generated JSON records 7,028 actual `regexp.Compile` validity observations
and 350,359 `MatchString` decisions. Cases cover every accepted property and category alias in positive,
negative, folded, and folded-negative forms; their interval endpoints and
neighbors; representative fold partners; Unicode 15.1/16/17 assignment/fold
sentinels; every nontrivial SimpleFold orbit in literal and complemented
spellings; deterministic syntax compositions; scoped flags; and
surrogate-only/range/complement/repetition cases.
Tests also replay the existing 53-case Go regex corpus without excluding the
previous Unicode/surrogate gaps.

This is author-generated differential coverage and source-derived Unicode
equivalence, not an independent review or full Go-regexp parity. The remaining
singleton-surrogate optimizer cases above are an explicit valid-Go gap, and
Go parser resource boundaries are not reproduced exactly. The Rust backend retains its finite compiled-size/nesting limits;
the adapter bounds translated size at 16 MiB, open groups at 4096, and
repeated wrapping of one atom below the backend nesting ceiling. These
resource-limit cases fail closed with an explicit `Unsupported` result rather
than authorizing a broader rule. Go's much larger regexp resource limits are
not reproduced. Inputs containing invalid UTF-8 cannot enter this `&str` API;
Go byte-string matching on such inputs is not claimed. Whole-policy caller
wiring and changes to old unsupported-case assertions are the integration
owner's responsibility; this standalone helper does not itself change active
policy behavior.

## Author validation receipt (2026-10-03)

The isolated, pre-integration helper checks used Rust 1.90.0 and the migration's
already cached, locked `regex` 1.13.1 / serde / serde_json rlibs. They did not
edit shared manifests, existing policy code, or module exports.

- Standalone `rustc --edition 2024 --test ... -D warnings`: passed.
- Standalone `clippy-driver --edition 2024 --test ... -D warnings`: passed.
- The resulting binary with `--test-threads=2`: **7 passed**, 0 failed (6.80s on
  the final post-lint source). The new corpus verifies 349,028 exact match
  decisions plus 61 rejected patterns. Its 29 enumerated valid-Go optimizer
  gaps verify `Unsupported`; their 1,331 Go decisions are recorded, not claimed
  as matched by Rust. The earlier 53-pattern corpus also passes without its
  historical Unicode/surrogate exclusions.
- `rustfmt --edition 2024 --config skip_children=true --check` on the helper,
  tests, and generated tables: passed.
- Two isolated generator runs, comparing `tables.rs`, `GO-LICENSE`,
  `go_oracle.json`, and `provenance.json`: byte-identical and identical to the
  checked-in generated outputs. `SHA256SUMS` records the data/generator hashes.

The first differential run exposed the singleton-surrogate U+FFFD exception;
that discrepancy was fixed within the bounded source-proven subset, and the
remaining cases are now explicit gaps. A clippy `collapsible_if` finding was
fixed before the final test run. These are implementation-author test receipts,
not an independent review or whole-policy/Hub acceptance.

To reproduce the standalone check before module integration, select the
already built locked rlibs (`REGEX_RLIB`, `SERDE_RLIB`, `SERDE_JSON_RLIB`) and
their shared dependency directory (`DEPS`), then run from the repository root:

```sh
tmp=$(mktemp -d)
printf '#[path = "%s/rust/src/approval/policy/go_regex.rs"]\nmod go_regex;\n' \
  "$PWD" > "$tmp/tests.rs"
"$RUSTC" --edition 2024 --test "$tmp/tests.rs" -o "$tmp/tests" \
  -L "dependency=$DEPS" --extern "regex=$REGEX_RLIB" \
  --extern "serde=$SERDE_RLIB" --extern "serde_json=$SERDE_JSON_RLIB" -D warnings
"$tmp/tests" --test-threads=2
```

Use the sibling `clippy-driver` executable in place of `RUSTC` for the lint
receipt. This is a focused helper check; the integration owner must run the
normal package tests and clippy after registration.

Integration updates required in the existing policy tests: remove the Han
load-case exclusion; replay all historical regex observations regardless of
their old `unsupported` annotations; replace the obsolete “Han unsupported”
assertion with an optimizer-gap warning case. Preserve the existing warnings,
hard blocks, policy store, and caller behavior. Include `GO-LICENSE` in binary
redistribution notices.

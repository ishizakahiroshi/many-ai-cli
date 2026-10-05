# Pinned Go string quoting

`oracle.go` is a synthetic standard-library-only oracle for
`strconv.Quote(string)` and `strconv.IsPrint(rune)`. It requires Go 1.26.8 with
Unicode 15.0.0 and records migration baseline
`21d0bc7935a2c4696fb89ccff2e324157a528c2d`. It imports no application code,
reads no user data, and starts no application, provider, shell command, or Hub.
Its shell-looking strings are inert test data.

Sources are the pinned toolchain's `src/strconv/quote.go`,
`src/strconv/isprint.go`, and Unicode version exported by `unicode.Version`.
The generated range data retains a reference to the existing Go BSD license at
`rust/src/approval/policy/go_regex/GO-LICENSE`.

## Contract and coverage

The API is `crate::proto::go_quote::quote(&str) -> String`. It matches Go's normal
`Quote`, including the surrounding double quotes, valid UTF-8 input, printable
Unicode, short control escapes, lowercase hexadecimal digits, two-digit `\x`,
four-digit `\u`, and eight-digit `\U` escapes. Single quotes and backticks do not
need escaping in a double-quoted Go string. It does not perform JSON encoding,
shell escaping, rune quoting, ASCII-only quoting, or invalid-UTF-8 byte quoting.
No wire decoding or parsing behavior is changed.

`oracle.json` contains 184 named exact-output cases, covering:

- Empty strings, every ASCII value, C0/C1 controls, DEL, quotes and backslashes
- Printable letters, marks (including spacing/combining/enclosing marks), numbers,
  punctuation, symbols, multilingual text, and emoji
- Non-ASCII spaces, format controls, bidi controls, zero-width joiners and BOM
- BMP/supplementary private-use characters, unassigned scalars, noncharacters,
  replacement/object-replacement characters, tags and variation selectors
- Unicode 15 assignments and later Unicode 15.1/16/17 assignments that the
  baseline still treats as unassigned
- Synthetic multiline subscription/profile/error details and handoff text

The same oracle records the SHA-256 of concatenated UTF-8 `strconv.Quote` outputs
for every valid Unicode scalar in ascending order, each scalar quoted separately.
Surrogate code points are omitted. There are 1,112,064 valid scalars and 148,998
printable scalars. The Rust exhaustive test calculates the same digest, proving
the exact scalar-by-scalar output independently of the chosen range encoding.
The targeted multi-scalar cases additionally exercise concatenation and escaping
interactions. This digest is a compact regression oracle, not a security proof.

`rust/src/proto/go_quote_data.rs` has 710 merged, inclusive, non-ASCII printable
ranges generated directly from the pinned `strconv.IsPrint`. ASCII is handled
explicitly in Rust. This avoids dependence on Rust's or regex's Unicode version.

## Reproduce offline

From the repository root, using the already provisioned pinned toolchain:

```sh
(
  set -eu
  TOOLS=/workspace/shared/many-ai-rust-tools
  export GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off
  export GOCACHE="$TOOLS/go-cache" GOMODCACHE="$TOOLS/go-mod"
  export GOPATH="$TOOLS/gopath"
  "$TOOLS/go/bin/go" run ./rust/tests/fixtures/foundation/go-quote/oracle.go \
    > rust/tests/fixtures/foundation/go-quote/oracle.json
  "$TOOLS/go/bin/go" run ./rust/tests/fixtures/foundation/go-quote/oracle.go -ranges \
    > rust/src/proto/go_quote_data.rs
  "$TOOLS/rust-1.90.0/bin/rustfmt" --edition 2024 rust/src/proto/go_quote.rs
)
```

The oracle fails rather than silently regenerating data with a different Go or
Unicode version. Rust tests, after the integration owner exports the module and
grants the serialized Cargo slot:

```sh
cargo test --offline --locked --manifest-path rust/Cargo.toml --lib proto::go_quote::tests
```

## Existing helper inventory at introduction

Only the new shared helper and this fixture directory are owned by this change;
the integration owner rewires the existing consumers separately.

- `rust/src/cli.rs::go_quote` uses Rust controls/whitespace and four selected format
  characters. It misses other format characters (for example U+00AD/U+200E),
  private-use characters and unassigned scalars.
- `rust/src/launcher/profile.rs::quote` uses regex `L/M/N/P/S` plus ASCII space.
- `rust/src/approval/policy.rs::quote_id` uses the complement of those regex
  categories. Its nearby `quote_meta` is regex-literal escaping, a separate
  contract that must not be replaced by this string-literal helper.
- `rust/src/orchestration/handoff.rs::go_quote` and
  `rust/src/orchestration/child_launch.rs::go_quote` use the same regex category
  complement.

All five string-literal helpers already use Go's ASCII escape spellings and
lowercase fixed-width Unicode escapes. The four regex-based versions generally
handle non-printable categories correctly, but newer Unicode tables can leave
newly assigned scalars literal where the pinned Go oracle emits an escape.

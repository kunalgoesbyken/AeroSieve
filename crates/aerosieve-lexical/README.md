# aerosieve-lexical

Single-pass, leftmost-longest, non-overlapping text normalization for code-mixed
(Hinglish) transcripts. Rules are declared in YAML and compiled once at startup.

## Why not a chain of `replace_all`?

The obvious implementation runs each rule's `replace_all` over the output of the
previous rule. That is what v1 of this crate did, and it is unsafe:

- **Corruptions cascade.** Text that rule *N* inserts becomes input that rule
  *N+1* can match and rewrite. With `₹500 -> 500 rupaye` followed by any rule
  matching `(\d+) rupaye`, the second rule fires on text the first one just
  wrote. The original span is gone; there is no way to recover it.
- **The result depends on rule order.** Reordering two unrelated-looking rules
  in the YAML silently changes the output.
- **A greedy rule starves everything.** v1 shipped `\s+ -> ' '` as an ordinary
  rule, so it ran on literally every input and rewrote the spacing every other
  rule depended on.

NVIDIA's NeMo inverse-text-normalization work (["NeMo Inverse Text
Normalization: From Development To Production", arXiv:2104.05055][nemo],
Interspeech 2021) uses hand-written WFST grammars for exactly this reason: the
task has "extremely low tolerance to unrecoverable errors." A full WFST stack is
out of scope here, but a single-pass disjoint-span matcher captures most of the
correctness win at a fraction of the complexity.

[nemo]: https://arxiv.org/abs/2104.05055

## How v2 works

```
text ──► Aho-Corasick pre-filter ──► candidate rules
                                          │
                                          ▼
                       collect ALL matches against the ORIGINAL text
                                          │
                                          ▼
                    sort: start asc, length desc, priority desc, index asc
                                          │
                                          ▼
                   greedily accept non-overlapping spans (leftmost-longest)
                                          │
                                          ▼
                   ONE pass over the original, splicing accepted spans
                                          │
                                          ▼
                        explicit whitespace collapse + trim
```

**No rule ever sees another rule's output.** Every match is computed against the
original input, so a replacement can never be re-matched, and the result does
not depend on the order rules appear in the YAML (except as an explicit final
tiebreak between two matches with identical start, length, and priority).

### Conflict resolution

Candidate matches are sorted by:

1. **start ascending** — leftmost wins
2. **length descending** — at the same start, longest wins
3. **priority descending** — at the same start *and* length, higher `priority`
   wins
4. **rule index ascending** — deterministic final tiebreak (YAML order)

Then they are accepted greedily: a match is applied only if it does not overlap
a span already accepted. `priority` therefore only matters for exact-length
collisions — do not use it to express "this rule is more specific", express that
in the pattern so longest-match handles it.

### Aho-Corasick pre-filter

Compiling the ruleset extracts a literal alphabetic keyword from each pattern
where one exists, and builds a single Aho-Corasick automaton over those
keywords. On each input, one automaton pass says which keyword-gated rules are
worth running at all; patterns with no extractable literal (e.g. `\d+`) are in
an `always_check` set and always run.

If the pre-filter matches nothing, **only the `always_check` rules run** — v1
fell back to running *every* rule in that case, inverting the point of the
optimization.

### Whitespace

Whitespace collapsing (runs of whitespace to one space, then trim) is an
explicit final step, not a rule. It cannot compete for spans and therefore
cannot starve a real rule. Toggle it with
`RuleEngine::with_whitespace_normalization(false)`.

## Usage

```rust
use aerosieve_lexical::RuleEngine;
use std::path::Path;

let engine = RuleEngine::from_yaml_file(Path::new("rules/default.yaml"))?;
let out = engine.normalize("yeh ₹500 hai");
assert!(out.normalized.contains("rupaye"));
// out.original      — the untouched input
// out.rules_applied — categories (or patterns) that actually fired
# Ok::<(), aerosieve_lexical::LexicalError>(())
```

## Rule format

```yaml
version: 2
rules:
  - pattern: '₹\s*(\d+)'      # regex, matched against the ORIGINAL text
    replacement: '$1 rupaye'  # capture expansion: $1, ${name}
    action: Replace           # Replace | Remove | Prefix | Suffix
    category: currency        # reported in rules_applied
    priority: 20              # tiebreak for same-start, same-length matches
```

Actions:

| Action    | Effect on the matched span                      |
|-----------|-------------------------------------------------|
| `Replace` | Replaced by the expanded replacement            |
| `Remove`  | Deleted                                         |
| `Prefix`  | Replacement inserted before, match kept         |
| `Suffix`  | Replacement inserted after, match kept          |

`Format` is accepted as a backwards-compatible alias for `Replace` (v1 had it as
a separate variant that behaved identically).

Patterns use the [`regex`][regex] crate, which has no backreferences and no
lookaround. Zero-width matches are skipped. Match offsets are always on UTF-8
char boundaries, so rules adjacent to multi-byte characters (`₹`, Devanagari,
emoji) are safe.

[regex]: https://docs.rs/regex

## Errors

`LexicalError` covers `Io` (unreadable rule file), `Yaml` (malformed YAML), and
`Regex` (an uncompilable pattern). An invalid pattern is a load-time error, never
a panic at normalize time.

## Testing and benchmarks

```
cargo test  -p aerosieve-lexical
cargo bench -p aerosieve-lexical
```

The benchmark covers a short Hinglish sentence, a long paragraph, a no-match
string (to show the pre-filter rejecting keyword-gated rules), and ruleset
compilation.

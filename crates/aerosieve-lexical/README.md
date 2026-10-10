# aerosieve-lexical

Rule-driven transcript normalization in a single pass. Rules are regexes declared in YAML and compiled once.

```rust
use aerosieve_lexical::RuleEngine;
use std::path::Path;

let engine = RuleEngine::from_yaml_file(Path::new("rules/hinglish.yaml"))?;
let normalized = engine.normalize("Dr. Sharma ne ₹500 diye");

assert_eq!(normalized.text, "doctor Sharma ne 500 rupaye diye");
assert_eq!(normalized.rules_applied, ["abbreviations", "currency"]);
```

## Why not a chain of `replace_all`?

Running each rule over the previous rule's output makes the result depend on rule order, and lets one rule rewrite text another just inserted: `dr → doctor` followed by `doctor → PHYSICIAN` yields `PHYSICIAN`.

Here every rule matches the **original** text:

1. Collect every match of every rule.
2. Resolve overlaps: leftmost wins, then longest, then higher `priority`, then the rule listed first.
3. Splice the surviving, disjoint matches into the text in one pass.
4. Collapse runs of whitespace to single spaces and trim the ends.

Whitespace collapsing is a fixed final step rather than a rule, because a `\s+` rule would compete with every other rule for spans. Disable it with `RuleEngine::with_whitespace_normalization(false)`.

## Rule format

```yaml
version: 2
rules:
  - pattern: '₹\s*(\d+)'      # regex, matched against the original text
    replacement: '$1 rupaye'  # capture expansion: $1, ${name}
    action: Replace           # Replace (default) | Remove | Prefix | Suffix
    category: currency        # reported in rules_applied; defaults to the pattern
    priority: 0               # tiebreak for matches with the same start and length
```

Only `pattern` is required. Unknown fields and unsupported versions are rejected at load time.

| Action | Effect |
|---|---|
| `Replace` | The match is replaced by the expanded replacement |
| `Remove` | The match is deleted |
| `Prefix` | The replacement is inserted before the match |
| `Suffix` | The replacement is inserted after the match |

Things to know when writing rules:

- Patterns use the [`regex`](https://docs.rs/regex) crate: no backreferences, no lookaround. Use `(?i)` for case-insensitive matching.
- Write `${1}s`, not `$1s`. The latter refers to a capture group named `1s`, which expands to nothing.
- To make trailing punctuation optional after a word, write `\bdr\b\.?`. The pattern `\bdr\.?\b` never consumes the period when a space follows it.
- Zero-width matches are ignored.

## Shipped rules

`rules/hinglish.yaml` normalizes romanized Hindi-English transcripts: currency, Indian number scales, dates, abbreviations, spelling variants, markup, and disfluencies. It is one language's ruleset, not a default for all transcripts. `tests/hinglish_rules.rs` pins its behaviour on whole sentences.

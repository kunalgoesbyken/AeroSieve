//! Single-pass, leftmost-longest, non-overlapping text normalization.
//!
//! # Why not chained `replace_all`?
//!
//! v1 of this crate applied rules as an ordered chain: rule *N* ran
//! `replace_all` over the *output* of rule *N-1*. That makes the result
//! order-dependent and lets corruptions cascade — text that one rule inserted
//! becomes input that a later rule can match and mangle, with no way to
//! recover the original. NeMo's inverse-text-normalization work
//! (arXiv 2104.05055) uses hand-written WFST grammars precisely because this
//! task has "extremely low tolerance to unrecoverable errors."
//!
//! v2 never lets a rule see another rule's output:
//!
//! 1. An Aho-Corasick keyword pre-filter narrows the candidate rule set.
//! 2. Every candidate rule collects *all* of its matches against the
//!    **original** text, recording byte spans.
//! 3. Conflicts are resolved by sorting on
//!    `(start asc, length desc, priority desc, rule index asc)` and greedily
//!    accepting a match only when it does not overlap an already-accepted
//!    span. That is leftmost-longest with an explicit priority tiebreak.
//! 4. The output is assembled in a single pass over the original text,
//!    splicing replacements into the disjoint accepted spans.
//!
//! Whitespace collapsing is deliberately *not* a rule — it would match
//! everywhere and starve every other rule in step 3. It runs as an explicit
//! final step instead.

use aho_corasick::AhoCorasick;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

/// A deserializable set of normalization rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSet {
    pub version: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// A single normalization rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Regex pattern, matched against the original text only.
    pub pattern: String,
    /// Replacement template. `$1`, `${name}` etc. are expanded from captures.
    pub replacement: String,
    #[serde(default)]
    pub action: RuleAction,
    #[serde(default)]
    pub category: String,
    /// Higher wins when two matches have the same start and the same length.
    #[serde(default)]
    pub priority: i32,
}

/// What to do with a matched span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RuleAction {
    /// Replace the matched span with the expanded replacement.
    #[default]
    Replace,
    /// Backwards-compatible alias for [`RuleAction::Replace`]. v1 had a
    /// `Format` variant that behaved identically; existing YAML keeps working.
    #[serde(alias = "format")]
    Format,
    /// Delete the matched span.
    Remove,
    /// Insert the replacement *before* the matched span, keeping the match.
    Prefix,
    /// Insert the replacement *after* the matched span, keeping the match.
    Suffix,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LexicalError {
    #[error("failed to read rule file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse rule YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("invalid rule regex: {0}")]
    Regex(#[from] regex::Error),
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CompiledRule {
    action: RuleAction,
    regex: Regex,
    replacement: String,
    category: String,
    priority: i32,
}

impl CompiledRule {
    /// Label reported in [`NormalizedText::rules_applied`].
    fn label(&self) -> &str {
        if self.category.is_empty() {
            self.regex.as_str()
        } else {
            &self.category
        }
    }
}

/// A candidate match against the original text, before conflict resolution.
#[derive(Debug)]
struct Candidate<'t> {
    start: usize,
    end: usize,
    rule_idx: usize,
    caps: regex::Captures<'t>,
}

/// Compiled, immutable rule engine.
#[derive(Debug)]
pub struct RuleEngine {
    rules: Vec<CompiledRule>,
    /// Aho-Corasick over literal keywords extracted from rule patterns.
    keywords: Option<AhoCorasick>,
    /// `keyword_map[pattern_id]` -> rule indices gated behind that keyword.
    keyword_map: Vec<Vec<usize>>,
    /// Rules with no extractable literal keyword; always candidates.
    always_check: Vec<usize>,
    /// Collapse whitespace runs and trim as an explicit final step.
    normalize_whitespace: bool,
}

#[derive(Debug, Clone)]
pub struct NormalizedText {
    pub original: String,
    pub normalized: String,
    /// Categories (or patterns, when a rule has no category) actually applied.
    pub rules_applied: Vec<String>,
}

impl RuleEngine {
    pub fn from_yaml_file(path: &Path) -> Result<Self, LexicalError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_yaml(&content)
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, LexicalError> {
        let rule_set: RuleSet = serde_yaml::from_str(yaml)?;
        Self::from_rule_set(&rule_set)
    }

    pub fn from_rule_set(rule_set: &RuleSet) -> Result<Self, LexicalError> {
        let mut rules = Vec::with_capacity(rule_set.rules.len());
        let mut by_keyword: HashMap<String, Vec<usize>> = HashMap::new();
        let mut always_check = Vec::new();

        for (i, rule) in rule_set.rules.iter().enumerate() {
            let regex = Regex::new(&rule.pattern)?;
            rules.push(CompiledRule {
                action: rule.action,
                regex,
                replacement: rule.replacement.clone(),
                category: rule.category.clone(),
                priority: rule.priority,
            });

            match extract_keyword(&rule.pattern) {
                Some(keyword) => by_keyword.entry(keyword).or_default().push(i),
                None => always_check.push(i),
            }
        }

        let keywords: Vec<String> = by_keyword.keys().cloned().collect();
        let ac = if keywords.is_empty() {
            None
        } else {
            Some(
                AhoCorasick::builder()
                    .ascii_case_insensitive(true)
                    // Standard semantics + overlapping iteration: two keywords
                    // that start at the same offset (e.g. "foo" and "foobar")
                    // must BOTH be reported, or the pre-filter silently drops
                    // a rule that would have won conflict resolution.
                    .match_kind(aho_corasick::MatchKind::Standard)
                    .build(&keywords)
                    .map_err(|e| LexicalError::Regex(regex::Error::Syntax(e.to_string())))?,
            )
        };

        let keyword_map: Vec<Vec<usize>> = keywords
            .iter()
            .map(|k| by_keyword.get(k).cloned().unwrap_or_default())
            .collect();

        Ok(Self {
            rules,
            keywords: ac,
            keyword_map,
            always_check,
            normalize_whitespace: true,
        })
    }

    /// An engine with no rules. Still collapses whitespace.
    pub fn empty() -> Self {
        Self {
            rules: Vec::new(),
            keywords: None,
            keyword_map: Vec::new(),
            always_check: Vec::new(),
            normalize_whitespace: true,
        }
    }

    /// Enable or disable the final whitespace-collapsing step (default: on).
    pub fn with_whitespace_normalization(mut self, enabled: bool) -> Self {
        self.normalize_whitespace = enabled;
        self
    }

    pub fn normalize_whitespace(&self) -> bool {
        self.normalize_whitespace
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Normalize `text` in a single pass over disjoint, leftmost-longest spans.
    pub fn normalize(&self, text: &str) -> NormalizedText {
        let candidate_rules = self.candidate_rules(text);

        // Step 2: collect every match of every candidate rule against the
        // ORIGINAL text. Nothing here ever sees another rule's output.
        let mut candidates: Vec<Candidate<'_>> = Vec::new();
        for &idx in &candidate_rules {
            for caps in self.rules[idx].regex.captures_iter(text) {
                let m = caps.get(0).expect("group 0 always present");
                // Skip zero-width matches: they cannot be spliced meaningfully
                // and would let an `Ok`-empty pattern fire at every position.
                if m.start() == m.end() {
                    continue;
                }
                candidates.push(Candidate {
                    start: m.start(),
                    end: m.end(),
                    rule_idx: idx,
                    caps,
                });
            }
        }

        // Step 3: leftmost-longest conflict resolution.
        // start asc, length desc, priority desc, rule index asc.
        candidates.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then_with(|| (b.end - b.start).cmp(&(a.end - a.start)))
                .then_with(|| {
                    self.rules[b.rule_idx]
                        .priority
                        .cmp(&self.rules[a.rule_idx].priority)
                })
                .then_with(|| a.rule_idx.cmp(&b.rule_idx))
        });

        // Step 4: one pass over the original, splicing accepted spans.
        let mut out = String::with_capacity(text.len() + text.len() / 4);
        let mut rules_applied: Vec<String> = Vec::new();
        let mut cursor = 0usize;

        for cand in &candidates {
            // Greedy accept: skip anything overlapping an accepted span.
            // Sorted by start, so `cursor` is the end of the last accepted span.
            if cand.start < cursor {
                continue;
            }
            // Byte-slicing is char-boundary safe: regex match offsets always
            // land on UTF-8 boundaries.
            out.push_str(&text[cursor..cand.start]);

            let rule = &self.rules[cand.rule_idx];
            let matched = &text[cand.start..cand.end];
            match rule.action {
                RuleAction::Remove => {}
                RuleAction::Replace | RuleAction::Format => {
                    cand.caps.expand(&rule.replacement, &mut out);
                }
                RuleAction::Prefix => {
                    cand.caps.expand(&rule.replacement, &mut out);
                    out.push_str(matched);
                }
                RuleAction::Suffix => {
                    out.push_str(matched);
                    cand.caps.expand(&rule.replacement, &mut out);
                }
            }

            let label = rule.label();
            if !rules_applied.iter().any(|r| r == label) {
                rules_applied.push(label.to_string());
            }
            cursor = cand.end;
        }
        out.push_str(&text[cursor..]);

        // Step 5: whitespace collapsing, explicit and last. Never competes for
        // spans, so it can never starve a real rule.
        let normalized = if self.normalize_whitespace {
            collapse_whitespace(&out)
        } else {
            out
        };

        NormalizedText {
            original: text.to_string(),
            normalized,
            rules_applied,
        }
    }

    /// Aho-Corasick pre-filter: which rules are worth running at all.
    ///
    /// If the pre-filter matches nothing, only `always_check` rules run.
    /// (v1 fell back to *all* rules here, inverting the optimization.)
    fn candidate_rules(&self, text: &str) -> Vec<usize> {
        let mut candidates = Vec::with_capacity(self.always_check.len() + 8);
        if let Some(ref ac) = self.keywords {
            for mat in ac.find_overlapping_iter(text) {
                let pattern_id = mat.pattern().as_usize();
                if let Some(indices) = self.keyword_map.get(pattern_id) {
                    candidates.extend_from_slice(indices);
                }
            }
        }
        candidates.extend_from_slice(&self.always_check);
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

/// Collapse runs of whitespace to a single space and trim the ends.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Pull a **required** literal substring out of a regex pattern, for use as an
/// Aho-Corasick pre-filter key. Returns `None` when no literal is guaranteed to
/// appear in every match — those rules go into `always_check` and always run.
///
/// This must be conservative: a keyword that is *not* actually required would
/// make the pre-filter skip a rule that should have fired. v1's version was
/// wrong in exactly that way — it mapped every non-alphanumeric byte to a
/// space, so `\bhello\b` yielded the keyword `bhello` (from the escape `\b`),
/// which never occurs in real text. v1 masked the bug by falling back to
/// running *all* rules whenever the pre-filter matched nothing; with that
/// fallback removed, the extractor has to be correct.
///
/// Rules honoured here:
/// - `\x` escapes contribute nothing (`\b`, `\d`, `\s`, `\w`, `\.` ...).
/// - Anything inside `(`...`)`, `[`...`]`, or `{`...`}` is skipped — an
///   alternation or class member is not required.
/// - A literal run is disqualified if the character right after it is a
///   quantifier (`?`, `*`) that could drop its last character; the run is
///   truncated by one instead.
/// - Only runs of >= 3 alphabetic characters qualify, so the automaton keeps
///   useful selectivity.
fn extract_keyword(pattern: &str) -> Option<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut best: Option<String> = None;
    let mut run = String::new();
    let mut i = 0usize;
    let mut depth_group = 0usize;

    // Close over "a finished literal run" so every exit path handles it once.
    fn consider(run: &mut String, best: &mut Option<String>) {
        if run.chars().count() >= 3 {
            let lower = run.to_lowercase();
            if best.as_ref().is_none_or(|b| lower.len() > b.len()) {
                *best = Some(lower);
            }
        }
        run.clear();
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                // Escape: consumes the next char and contributes no literal we
                // can trust (`\b` is an anchor, `\.` is literal but breaking
                // the run here is merely conservative).
                consider(&mut run, &mut best);
                i += 2;
                continue;
            }
            '(' => {
                consider(&mut run, &mut best);
                depth_group += 1;
            }
            ')' => {
                depth_group = depth_group.saturating_sub(1);
            }
            '[' => {
                consider(&mut run, &mut best);
                // Skip the whole class.
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '{' => {
                // Repetition count on the previous char: it may be `{0,n}`, so
                // drop that char from the run to stay safe.
                run.pop();
                consider(&mut run, &mut best);
                while i < chars.len() && chars[i] != '}' {
                    i += 1;
                }
            }
            '?' | '*' => {
                // Previous char is optional.
                run.pop();
                consider(&mut run, &mut best);
            }
            '|' | '^' | '$' | '.' | '+' => {
                consider(&mut run, &mut best);
            }
            _ if depth_group > 0 => {
                // Inside a group: could be one branch of an alternation, so no
                // character in here is guaranteed.
                consider(&mut run, &mut best);
            }
            _ if c.is_alphabetic() => run.push(c),
            _ => consider(&mut run, &mut best),
        }
        i += 1;
    }
    consider(&mut run, &mut best);
    best
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(yaml: &str) -> RuleEngine {
        RuleEngine::from_yaml(yaml).expect("yaml should compile")
    }

    // -- ported v1 tests ---------------------------------------------------

    #[test]
    fn basic_replacement() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '₹\s*(\d+)'
    replacement: '$1 rupaye'
    action: Replace
    category: currency
"#,
        );
        let r = e.normalize("yeh \u{20B9}500 hai");
        assert_eq!(r.normalized, "yeh 500 rupaye hai");
        assert_eq!(r.rules_applied, vec!["currency"]);
    }

    #[test]
    fn multiple_disjoint_rules() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '₹\s*(\d+)'
    replacement: '$1 rupaye'
    category: currency
  - pattern: '(\d+)\s*km'
    replacement: '$1 kilometer'
    category: distance
"#,
        );
        let r = e.normalize("\u{20B9}100 ke liye 5km");
        assert_eq!(r.normalized, "100 rupaye ke liye 5 kilometer");
    }

    #[test]
    fn remove_action() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '<[^>]+>'
    replacement: ''
    action: Remove
    category: markup
"#,
        );
        assert_eq!(e.normalize("hello <b>world</b>").normalized, "hello world");
    }

    #[test]
    fn no_matching_rules() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '₹\d+'
    replacement: 'rupees'
    category: currency
"#,
        );
        let r = e.normalize("no currency here");
        assert_eq!(r.normalized, "no currency here");
        assert!(r.rules_applied.is_empty());
    }

    #[test]
    fn empty_engine_is_identity() {
        let e = RuleEngine::empty();
        assert_eq!(e.normalize("hello world").normalized, "hello world");
        assert_eq!(e.rule_count(), 0);
    }

    #[test]
    fn empty_ruleset_and_empty_text() {
        let e = engine("version: 1\nrules: []\n");
        assert_eq!(e.rule_count(), 0);
        assert_eq!(e.normalize("").normalized, "");
        assert_eq!(e.normalize("abc").normalized, "abc");
        assert_eq!(RuleEngine::empty().normalize("").normalized, "");
    }

    #[test]
    fn hindi_numerals_format_alias() {
        // `Format` is a v1 alias for Replace and must still deserialize.
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '\b(\d+)\s*(?:lakh|laakh)\b'
    replacement: '$1 lakh'
    action: Format
    category: weights
  - pattern: '\b(\d+)\s*(?:crore|karod)\b'
    replacement: '$1 crore'
    action: Format
    category: weights
"#,
        );
        let r = e.normalize("5 laakh rupaye aur 2 karod");
        assert!(r.normalized.contains("5 lakh"), "{}", r.normalized);
        assert!(r.normalized.contains("2 crore"), "{}", r.normalized);
    }

    #[test]
    fn prefix_action() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '\bhello\b'
    replacement: 'say '
    action: Prefix
    category: test
"#,
        );
        assert_eq!(e.normalize("hello world").normalized, "say hello world");
    }

    #[test]
    fn suffix_action() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '\bcompleted\b'
    replacement: ' done'
    action: Suffix
    category: test
"#,
        );
        assert_eq!(e.normalize("task completed").normalized, "task completed done");
    }

    #[test]
    fn default_rules_load_and_normalize_currency() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("rules/default.yaml");
        let e = RuleEngine::from_yaml_file(&path).expect("default rules must load");
        assert!(e.rule_count() > 0);
        let r = e.normalize("yeh \u{20B9}500 hai");
        assert!(r.normalized.contains("rupaye"), "{}", r.normalized);
    }

    // -- THE CASCADE TEST --------------------------------------------------

    /// The point of the v2 rewrite.
    ///
    /// Two rules, applied in YAML order:
    ///   R1: `₹\s*(\d+)`          -> `$1 rupaye`
    ///   R2: `\b(\d+)\s+rupaye\b` -> `$1 INR`      (an older/looser currency rule)
    ///
    /// v1 (chained `replace_all`, R1 then R2 over R1's OUTPUT):
    ///   "₹500"  --R1--> "500 rupaye"  --R2--> "500 INR"
    ///   Final:  "yeh 500 INR hai"   <-- WRONG. R2 matched text R1 just wrote.
    ///
    /// v2 (single pass over the ORIGINAL text):
    ///   Only R1 matches the original ("500 rupaye" does not exist in the input).
    ///   Final:  "yeh 500 rupaye hai"
    ///
    /// The v1 result is also order-dependent: swapping the two rules in the
    /// YAML changes the output. v2's result does not depend on rule order.
    #[test]
    fn cascade_rule_output_is_never_re_matched() {
        let yaml = r#"
version: 1
rules:
  - pattern: '₹\s*(\d+)'
    replacement: '$1 rupaye'
    category: currency_symbol
  - pattern: '\b(\d+)\s+rupaye\b'
    replacement: '$1 INR'
    category: currency_word
"#;
        let e = engine(yaml);
        let r = e.normalize("yeh \u{20B9}500 hai");

        // v1 would have produced "yeh 500 INR hai".
        assert_eq!(r.normalized, "yeh 500 rupaye hai");
        assert_eq!(r.rules_applied, vec!["currency_symbol"]);

        // And rule order does not matter in v2.
        let swapped = engine(
            r#"
version: 1
rules:
  - pattern: '\b(\d+)\s+rupaye\b'
    replacement: '$1 INR'
    category: currency_word
  - pattern: '₹\s*(\d+)'
    replacement: '$1 rupaye'
    category: currency_symbol
"#,
        );
        assert_eq!(swapped.normalize("yeh \u{20B9}500 hai").normalized, "yeh 500 rupaye hai");
    }

    #[test]
    fn replacement_text_is_not_re_matched_by_another_rule() {
        // R2's pattern matches exactly what R1 inserts. It must not fire.
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '\bdr\b'
    replacement: 'doctor'
    category: abbrev
  - pattern: '\bdoctor\b'
    replacement: 'PHYSICIAN'
    category: expand
"#,
        );
        let r = e.normalize("dr sharma");
        assert_eq!(r.normalized, "doctor sharma"); // v1: "PHYSICIAN sharma"
        assert_eq!(r.rules_applied, vec!["abbrev"]);
    }

    // -- conflict resolution ----------------------------------------------

    #[test]
    fn overlapping_matches_longest_wins() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: 'foo'
    replacement: 'SHORT'
    category: short
  - pattern: 'foobar'
    replacement: 'LONG'
    category: long
"#,
        );
        let r = e.normalize("xx foobar yy");
        assert_eq!(r.normalized, "xx LONG yy");
        assert_eq!(r.rules_applied, vec!["long"]);
    }

    #[test]
    fn equal_length_overlap_higher_priority_wins() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: 'ab[c]'
    replacement: 'LOW'
    category: low
    priority: 1
  - pattern: 'a[b]c'
    replacement: 'HIGH'
    category: high
    priority: 10
"#,
        );
        let r = e.normalize("xx abc yy");
        assert_eq!(r.normalized, "xx HIGH yy");
        assert_eq!(r.rules_applied, vec!["high"]);
    }

    #[test]
    fn equal_length_equal_priority_breaks_by_rule_order() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: 'ab[c]'
    replacement: 'FIRST'
    category: first
  - pattern: 'a[b]c'
    replacement: 'SECOND'
    category: second
"#,
        );
        assert_eq!(e.normalize("abc").normalized, "FIRST");
    }

    #[test]
    fn leftmost_wins_over_a_longer_later_match() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: 'ab'
    replacement: '[AB]'
    category: left
  - pattern: 'bcde'
    replacement: '[BCDE]'
    category: right
"#,
        );
        // "ab" starts at 0, "bcde" starts at 1 and overlaps -> leftmost wins.
        assert_eq!(e.normalize("abcde").normalized, "[AB]cde");
    }

    // -- captures ----------------------------------------------------------

    #[test]
    fn capture_expansion_works() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '(\d{1,2})[/-](\d{1,2})[/-](\d{2,4})'
    replacement: '$3-$2-$1'
    category: dates
"#,
        );
        assert_eq!(e.normalize("on 05/09/2026 ok").normalized, "on 2026-09-05 ok");
    }

    #[test]
    fn named_capture_expansion_works() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '(?P<num>\d+)\s*kg'
    replacement: '${num} kilogram'
    category: weights
"#,
        );
        assert_eq!(e.normalize("5kg aata").normalized, "5 kilogram aata");
    }

    // -- pre-filter --------------------------------------------------------

    #[test]
    fn prefilter_miss_runs_only_always_check_rules() {
        // `\d+` has no literal keyword -> always_check.
        // `rupaye` has a keyword -> gated behind the pre-filter.
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '\d+'
    replacement: 'N'
    category: numbers
  - pattern: 'rupaye'
    replacement: 'INR'
    category: currency
"#,
        );
        let r = e.normalize("abc 42 xyz");
        assert_eq!(r.normalized, "abc N xyz");
        assert_eq!(r.rules_applied, vec!["numbers"]);
        // Pre-filter hit still works.
        assert_eq!(e.normalize("5 rupaye").normalized, "N INR");
    }

    // -- whitespace --------------------------------------------------------

    #[test]
    fn whitespace_is_collapsed_as_a_final_step() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '<[^>]+>'
    replacement: ''
    action: Remove
    category: markup
"#,
        );
        assert_eq!(e.normalize("  a   <b>  c  ").normalized, "a c");
        assert_eq!(e.normalize("a\n\tb").normalized, "a b");
    }

    #[test]
    fn whitespace_normalization_can_be_disabled() {
        let e = engine("version: 1\nrules: []\n").with_whitespace_normalization(false);
        assert!(!e.normalize_whitespace());
        assert_eq!(e.normalize("  a   b  ").normalized, "  a   b  ");
    }

    #[test]
    fn whitespace_rule_does_not_starve_real_rules() {
        // In v1, `\s+ -> ' '` was a rule and ran on literally every input.
        // Here the only rules are real ones; whitespace is handled separately,
        // so the currency rule still wins its span.
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '₹\s*(\d+)'
    replacement: '$1 rupaye'
    category: currency
"#,
        );
        assert_eq!(e.normalize("  yeh   \u{20B9} 500   hai ").normalized, "yeh 500 rupaye hai");
    }

    // -- unicode safety ----------------------------------------------------

    #[test]
    fn unicode_multibyte_never_splits_or_panics() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '₹\s*(\d+)'
    replacement: '$1 rupaye'
    category: currency
  - pattern: '\bnamaste\b'
    replacement: 'नमस्ते'
    category: greeting
  - pattern: 'रुपये'
    replacement: 'rupaye'
    category: devanagari
"#,
        );
        let r = e.normalize("नमस्ते, yeh ₹500 aur 200 रुपये hai — ठीक है namaste");
        assert!(r.normalized.contains("500 rupaye"), "{}", r.normalized);
        assert!(r.normalized.contains("200 rupaye"), "{}", r.normalized);
        assert!(r.normalized.contains("नमस्ते"), "{}", r.normalized);
        assert!(r.normalized.is_char_boundary(0));
        // Round-trips as valid UTF-8 by construction; assert chars survive.
        assert!(r.normalized.chars().count() > 0);
    }

    #[test]
    fn unicode_rule_adjacent_to_multibyte_chars() {
        // A rule whose match butts directly against multi-byte neighbours.
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '(\d+)'
    replacement: '<$1>'
    category: numbers
"#,
        );
        let r = e.normalize("₹500₹ नमस्ते7ठीक");
        assert_eq!(r.normalized, "₹<500>₹ नमस्ते<7>ठीक");
    }

    #[test]
    fn emoji_and_combining_marks_survive() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '\bok\b'
    replacement: 'theek'
    category: ack
"#,
        );
        let r = e.normalize("ok 🎉 café ok");
        assert_eq!(r.normalized, "theek 🎉 café theek");
    }

    // -- errors ------------------------------------------------------------

    #[test]
    fn invalid_regex_is_an_error_not_a_panic() {
        let err = RuleEngine::from_yaml(
            r#"
version: 1
rules:
  - pattern: '([unclosed'
    replacement: 'x'
    category: broken
"#,
        )
        .expect_err("invalid regex must error");
        assert!(matches!(err, LexicalError::Regex(_)), "got {err:?}");
    }

    #[test]
    fn invalid_yaml_is_an_error() {
        let err = RuleEngine::from_yaml("version: [this is not a number\n")
            .expect_err("invalid yaml must error");
        assert!(matches!(err, LexicalError::Yaml(_)), "got {err:?}");
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = RuleEngine::from_yaml_file(Path::new("does/not/exist.yaml"))
            .expect_err("missing file must error");
        assert!(matches!(err, LexicalError::Io(_)), "got {err:?}");
    }

    // -- misc --------------------------------------------------------------

    #[test]
    fn zero_width_matches_are_skipped() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: 'x*'
    replacement: 'ZW'
    category: zero
"#,
        );
        // Only the real "xx" match applies; empty matches are ignored.
        assert_eq!(e.normalize("a xx b").normalized, "a ZW b");
    }

    #[test]
    fn rules_applied_is_deduplicated() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: '(\d+)'
    replacement: 'N'
    category: numbers
"#,
        );
        let r = e.normalize("1 2 3 4");
        assert_eq!(r.normalized, "N N N N");
        assert_eq!(r.rules_applied, vec!["numbers"]);
    }

    #[test]
    fn keyword_extraction_is_conservative() {
        // The v1 bug: `\bhello\b` must NOT yield "bhello".
        assert_eq!(extract_keyword(r"\bhello\b").as_deref(), Some("hello"));
        assert_eq!(extract_keyword(r"\brupaye\b").as_deref(), Some("rupaye"));
        // No guaranteed literal at all -> always_check.
        assert_eq!(extract_keyword(r"\d+"), None);
        assert_eq!(extract_keyword(r"\s*([.,!?;:])"), None);
        assert_eq!(extract_keyword(r"<[^>]+>"), None);
        // Alternation members are not required.
        assert_eq!(extract_keyword(r"\b(?:lakh|laakh)\b"), None);
        // Optional trailing char is dropped.
        assert_eq!(extract_keyword(r"\brupees?\b").as_deref(), Some("rupee"));
        // Picks the longest guaranteed run.
        assert_eq!(
            extract_keyword(r"\bab\s+government\b").as_deref(),
            Some("government")
        );
    }

    #[test]
    fn every_extracted_keyword_actually_occurs_in_its_pattern_matches() {
        // Property: if a rule is gated behind keyword K, then K must appear in
        // any text the rule matches. Verified against the shipped ruleset by
        // checking the keyword is a literal substring of the pattern source.
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("rules/default.yaml");
        let yaml = std::fs::read_to_string(&path).unwrap();
        let rs: RuleSet = serde_yaml::from_str(&yaml).unwrap();
        for rule in &rs.rules {
            if let Some(kw) = extract_keyword(&rule.pattern) {
                assert!(
                    rule.pattern.to_lowercase().contains(&kw),
                    "keyword {kw:?} not literal in pattern {:?}",
                    rule.pattern
                );
            }
        }
    }

    #[test]
    fn default_rules_normalize_a_realistic_sentence() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("rules/default.yaml");
        let e = RuleEngine::from_yaml_file(&path).unwrap();

        let r = e.normalize("  govt. dept. ne  Rs. 1200  diya , achchha  ");
        assert!(r.normalized.contains("government"), "{}", r.normalized);
        assert!(r.normalized.contains("department"), "{}", r.normalized);
        assert!(r.normalized.contains("1200 rupaye"), "{}", r.normalized);
        assert!(r.normalized.contains("achha"), "{}", r.normalized);
        // Whitespace step ran: no leading/trailing/double spaces.
        assert_eq!(r.normalized.trim(), r.normalized);
        assert!(!r.normalized.contains("  "), "{}", r.normalized);

        // The dangerous v1 rule is gone: a standalone `h` stays a standalone h.
        let r2 = e.normalize("mera naam h k sharma hai");
        assert!(r2.normalized.contains(" h "), "{}", r2.normalized);
    }

    #[test]
    fn default_rules_have_no_whitespace_only_rule() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("rules/default.yaml");
        let yaml = std::fs::read_to_string(&path).unwrap();
        let rs: RuleSet = serde_yaml::from_str(&yaml).unwrap();
        for rule in &rs.rules {
            assert_ne!(rule.pattern, r"\s+", "whitespace rule must be removed");
            assert_ne!(rule.pattern, r"^\s+|\s+$", "trim rule must be removed");
        }
    }

    #[test]
    fn original_is_preserved() {
        let e = engine(
            r#"
version: 1
rules:
  - pattern: 'a'
    replacement: 'b'
    category: t
"#,
        );
        let r = e.normalize("  a  ");
        assert_eq!(r.original, "  a  ");
        assert_eq!(r.normalized, "b");
    }
}

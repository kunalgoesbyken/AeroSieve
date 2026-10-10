//! Rule-driven transcript normalization in a single pass.
//!
//! Applying rules as a chain of `replace_all` calls lets each rule rewrite
//! the output of the ones before it, so the result depends on rule order and
//! one bad rewrite cascades. Here every rule matches the *original* text.
//! Overlapping matches are resolved leftmost first, then longest, then by
//! `priority`, then by rule order, and the surviving disjoint matches are
//! spliced into the text in one pass. Whitespace is collapsed afterwards.
//!
//! ```
//! use aerosieve_lexical::RuleEngine;
//!
//! let engine = RuleEngine::from_yaml(r#"
//! version: 2
//! rules:
//!   - pattern: '₹\s*(\d+)'
//!     replacement: '$1 rupaye'
//!     category: currency
//! "#)?;
//!
//! let normalized = engine.normalize("yeh  ₹500 hai");
//! assert_eq!(normalized.text, "yeh 500 rupaye hai");
//! assert_eq!(normalized.rules_applied, ["currency"]);
//! # Ok::<(), aerosieve_lexical::LexicalError>(())
//! ```

#[cfg(test)]
mod tests;

use std::cmp::Reverse;
use std::ops::Range;
use std::path::Path;

use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

/// The ruleset format version this engine reads.
pub const RULESET_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    pub version: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Regex in the syntax of the `regex` crate.
    pub pattern: String,
    /// Template expanded from the match's captures: `$1`, `${name}`. Write
    /// `${1}s`, not `$1s`, when a letter, digit, or underscore follows.
    #[serde(default)]
    pub replacement: String,
    #[serde(default)]
    pub action: RuleAction,
    /// Reported in [`NormalizedText::rules_applied`]; defaults to the pattern.
    #[serde(default)]
    pub category: String,
    /// Breaks ties between matches with the same start and length.
    #[serde(default)]
    pub priority: i32,
}

/// What a rule does with the text it matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RuleAction {
    /// Substitute the replacement for the match.
    #[default]
    #[serde(alias = "Format")]
    Replace,
    /// Delete the match.
    Remove,
    /// Insert the replacement before the match.
    Prefix,
    /// Insert the replacement after the match.
    Suffix,
}

#[derive(Debug, thiserror::Error)]
pub enum LexicalError {
    #[error("failed to read rule file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse rule YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("ruleset version {found} is not supported (expected {RULESET_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("invalid pattern {pattern:?}: {source}")]
    InvalidPattern {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedText {
    pub text: String,
    /// Labels of the rules that changed the text, in order of first use.
    pub rules_applied: Vec<String>,
}

#[derive(Debug)]
struct CompiledRule {
    regex: Regex,
    replacement: String,
    action: RuleAction,
    label: String,
    priority: i32,
}

impl CompiledRule {
    fn compile(rule: &Rule) -> Result<Self, LexicalError> {
        let regex = Regex::new(&rule.pattern).map_err(|source| LexicalError::InvalidPattern {
            pattern: rule.pattern.clone(),
            source,
        })?;
        let label = if rule.category.is_empty() {
            &rule.pattern
        } else {
            &rule.category
        };
        Ok(Self {
            regex,
            replacement: rule.replacement.clone(),
            action: rule.action,
            label: label.clone(),
            priority: rule.priority,
        })
    }

    fn write_rewrite(&self, captures: &Captures<'_>, output: &mut String) {
        let matched = &captures[0];
        match self.action {
            RuleAction::Remove => {}
            RuleAction::Replace => captures.expand(&self.replacement, output),
            RuleAction::Prefix => {
                captures.expand(&self.replacement, output);
                output.push_str(matched);
            }
            RuleAction::Suffix => {
                output.push_str(matched);
                captures.expand(&self.replacement, output);
            }
        }
    }
}

struct RuleMatch<'engine, 'text> {
    rule: &'engine CompiledRule,
    rule_index: usize,
    span: Range<usize>,
    captures: Captures<'text>,
}

/// A compiled, immutable ruleset.
#[derive(Debug)]
pub struct RuleEngine {
    rules: Vec<CompiledRule>,
    collapse_whitespace: bool,
}

impl RuleEngine {
    pub fn from_yaml_file(path: &Path) -> Result<Self, LexicalError> {
        Self::from_yaml(&std::fs::read_to_string(path)?)
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, LexicalError> {
        Self::from_rule_set(&serde_yaml::from_str(yaml)?)
    }

    pub fn from_rule_set(rule_set: &RuleSet) -> Result<Self, LexicalError> {
        if rule_set.version != RULESET_VERSION {
            return Err(LexicalError::UnsupportedVersion {
                found: rule_set.version,
            });
        }
        let rules = rule_set
            .rules
            .iter()
            .map(CompiledRule::compile)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            rules,
            collapse_whitespace: true,
        })
    }

    /// Choose whether runs of whitespace are collapsed to single spaces and
    /// the ends trimmed after the rules have run. On by default.
    pub fn with_whitespace_normalization(mut self, enabled: bool) -> Self {
        self.collapse_whitespace = enabled;
        self
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    pub fn normalize(&self, text: &str) -> NormalizedText {
        let mut matches = self.find_matches(text);
        matches.sort_by_key(|found| {
            (
                found.span.start,
                Reverse(found.span.end),
                Reverse(found.rule.priority),
                found.rule_index,
            )
        });

        let mut rewritten = String::with_capacity(text.len());
        let mut rules_applied: Vec<String> = Vec::new();
        let mut cursor = 0;
        for found in matches {
            if found.span.start < cursor {
                continue;
            }
            rewritten.push_str(&text[cursor..found.span.start]);
            found.rule.write_rewrite(&found.captures, &mut rewritten);
            cursor = found.span.end;

            if !rules_applied.contains(&found.rule.label) {
                rules_applied.push(found.rule.label.clone());
            }
        }
        rewritten.push_str(&text[cursor..]);

        NormalizedText {
            text: if self.collapse_whitespace {
                collapse_whitespace(&rewritten)
            } else {
                rewritten
            },
            rules_applied,
        }
    }

    /// Every non-empty match of every rule against `text`.
    fn find_matches<'engine, 'text>(
        &'engine self,
        text: &'text str,
    ) -> Vec<RuleMatch<'engine, 'text>> {
        self.rules
            .iter()
            .enumerate()
            .flat_map(|(rule_index, rule)| {
                rule.regex.captures_iter(text).filter_map(move |captures| {
                    Some(RuleMatch {
                        rule,
                        rule_index,
                        span: captures.get(0)?.range(),
                        captures,
                    })
                })
            })
            .filter(|found| !found.span.is_empty())
            .collect()
    }
}

fn collapse_whitespace(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    collapsed
}

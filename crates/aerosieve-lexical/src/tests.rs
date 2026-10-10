use super::*;

fn rule(pattern: &str, replacement: &str) -> Rule {
    Rule {
        pattern: pattern.to_string(),
        replacement: replacement.to_string(),
        action: RuleAction::Replace,
        category: replacement.to_string(),
        priority: 0,
    }
}

fn engine(rules: Vec<Rule>) -> RuleEngine {
    RuleEngine::from_rule_set(&RuleSet {
        version: RULESET_VERSION,
        rules,
    })
    .unwrap()
}

fn normalize(rules: Vec<Rule>, text: &str) -> String {
    engine(rules).normalize(text).text
}

#[test]
fn captures_are_expanded_into_the_replacement() {
    let numbered = rule(r"(\d{1,2})/(\d{1,2})/(\d{4})", "$3-$2-$1");
    assert_eq!(
        normalize(vec![numbered], "on 05/09/2026 ok"),
        "on 2026-09-05 ok"
    );

    let named = rule(r"(?P<amount>\d+)\s*kg", "${amount} kilogram");
    assert_eq!(normalize(vec![named], "5kg aata"), "5 kilogram aata");
}

#[test]
fn each_action_rewrites_the_match_differently() {
    let with_action = |action| Rule {
        action,
        ..rule("world", "+")
    };

    for (action, expected) in [
        (RuleAction::Replace, "hello +"),
        (RuleAction::Remove, "hello"),
        (RuleAction::Prefix, "hello +world"),
        (RuleAction::Suffix, "hello world+"),
    ] {
        assert_eq!(
            normalize(vec![with_action(action)], "hello world"),
            expected
        );
    }
}

#[test]
fn a_rule_never_matches_text_another_rule_inserted() {
    let rules = vec![rule(r"\bdr\b", "doctor"), rule(r"\bdoctor\b", "PHYSICIAN")];
    let reversed: Vec<Rule> = rules.iter().rev().cloned().collect();

    assert_eq!(normalize(rules, "dr sharma"), "doctor sharma");
    assert_eq!(normalize(reversed, "dr sharma"), "doctor sharma");
}

#[test]
fn every_branch_of_an_alternation_can_match() {
    let rules = vec![rule("hello|world", "X")];
    assert_eq!(normalize(rules, "world hello"), "X X");
}

#[test]
fn overlaps_resolve_leftmost_then_longest_then_priority_then_order() {
    let leftmost = vec![rule("bcde", "LATER"), rule("ab", "EARLIER")];
    assert_eq!(normalize(leftmost, "abcde"), "EARLIERcde");

    let longest = vec![rule("foo", "SHORT"), rule("foobar", "LONG")];
    assert_eq!(normalize(longest, "foobar"), "LONG");

    let prioritized = vec![
        rule("ab[c]", "LOW"),
        Rule {
            priority: 10,
            ..rule("a[b]c", "HIGH")
        },
    ];
    assert_eq!(normalize(prioritized, "abc"), "HIGH");

    let ordered = vec![rule("ab[c]", "FIRST"), rule("a[b]c", "SECOND")];
    assert_eq!(normalize(ordered, "abc"), "FIRST");
}

#[test]
fn case_insensitive_patterns_fold_unicode() {
    let rules = vec![rule(r"(?i)\bstraße\b", "street")];
    assert_eq!(normalize(rules, "die STRAßE"), "die street");
}

#[test]
fn multibyte_text_around_a_match_is_preserved() {
    let rules = vec![rule(r"\d+", "<$0>"), rule("रुपये", "rupaye")];
    assert_eq!(
        normalize(rules, "₹500₹ नमस्ते7ठीक 🎉 café 200 रुपये"),
        "₹<500>₹ नमस्ते<7>ठीक 🎉 café <200> rupaye"
    );
}

#[test]
fn zero_width_matches_are_ignored() {
    assert_eq!(normalize(vec![rule("x*", "Z")], "a xx b"), "a Z b");
}

#[test]
fn whitespace_is_collapsed_after_the_rules_run() {
    let strip_tags = Rule {
        action: RuleAction::Remove,
        ..rule("<[^>]+>", "")
    };
    assert_eq!(
        normalize(vec![strip_tags], "  a \u{00A0} <b>\n\tc  "),
        "a c"
    );
}

#[test]
fn whitespace_collapsing_can_be_disabled() {
    let engine = engine(Vec::new()).with_whitespace_normalization(false);
    assert_eq!(engine.normalize("  a   b  ").text, "  a   b  ");
}

#[test]
fn rules_applied_lists_each_label_once_in_order_of_first_use() {
    let unlabelled = Rule {
        category: String::new(),
        ..rule("[a-z]+", "W")
    };
    let rules = vec![rule(r"\d+", "number"), unlabelled];

    let normalized = engine(rules).normalize("ab 1 cd 2");

    assert_eq!(normalized.rules_applied, ["[a-z]+", "number"]);
}

#[test]
fn text_without_matches_is_returned_unchanged() {
    let normalized = engine(vec![rule(r"\d+", "N")]).normalize("no digits");
    assert_eq!(normalized.text, "no digits");
    assert!(normalized.rules_applied.is_empty());
}

#[test]
fn yaml_fields_other_than_pattern_are_optional() {
    let engine = RuleEngine::from_yaml("version: 2\nrules:\n  - pattern: 'x'\n").unwrap();
    assert_eq!(engine.rule_count(), 1);
    assert_eq!(engine.normalize("axb").text, "ab");
}

#[test]
fn legacy_format_action_reads_as_replace() {
    let rule: Rule = serde_saphyr::from_str("{ pattern: x, action: Format }").unwrap();
    assert_eq!(rule.action, RuleAction::Replace);
}

#[test]
fn a_misspelled_field_is_rejected() {
    let yaml = "version: 2\nrules:\n  - pattern: 'x'\n    replacment: 'y'\n";
    assert!(matches!(
        RuleEngine::from_yaml(yaml),
        Err(LexicalError::Yaml(_))
    ));
}

#[test]
fn an_unsupported_version_is_rejected() {
    assert!(matches!(
        RuleEngine::from_yaml("version: 1\nrules: []\n"),
        Err(LexicalError::UnsupportedVersion { found: 1 })
    ));
}

#[test]
fn an_invalid_pattern_is_reported_with_its_source() {
    let result = RuleEngine::from_rule_set(&RuleSet {
        version: RULESET_VERSION,
        rules: vec![rule("([unclosed", "x")],
    });
    assert!(matches!(
        result,
        Err(LexicalError::InvalidPattern { pattern, .. }) if pattern == "([unclosed"
    ));
}

#[test]
fn a_missing_rule_file_is_an_io_error() {
    assert!(matches!(
        RuleEngine::from_yaml_file(Path::new("does/not/exist.yaml")),
        Err(LexicalError::Io(_))
    ));
}

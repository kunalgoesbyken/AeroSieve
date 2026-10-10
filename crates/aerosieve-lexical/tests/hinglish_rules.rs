//! The shipped Hinglish ruleset, checked against whole sentences.

use std::path::PathBuf;

use aerosieve_lexical::RuleEngine;

fn engine() -> RuleEngine {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rules/hinglish.yaml");
    RuleEngine::from_yaml_file(&path).unwrap()
}

#[test]
fn sentences_normalize_as_expected() {
    let engine = engine();
    let cases = [
        ("yeh ₹500 hai", "yeh 500 rupaye hai"),
        (
            "Rs. 1,200 aur Rs50 aur 20 rupees",
            "1,200 rupaye aur 50 rupaye aur 20 rupaye",
        ),
        (
            "5 laakh ya 2 karod ya 3 hazar",
            "5 lakh ya 2 crore ya 3 hazaar",
        ),
        (
            "Dr. Sharma vs. Prof Gupta",
            "doctor Sharma versus professor Gupta",
        ),
        (
            "govt. dept. ne diya , achchha  ",
            "government department ne diya, achha",
        ),
        ("15th March ko 05/09/2026 ko", "15 March ko 05-09-2026 ko"),
        (
            "main ja raha hu, bohot kaam hein",
            "main ja raha hoon, bahut kaam hai",
        ),
        ("a<br>b&nbsp;c", "a b c"),
        ("umm theek hai hmm", "theek hai"),
        ("ठीक है ।", "ठीक है।"),
    ];

    for (input, expected) in cases {
        assert_eq!(engine.normalize(input).text, expected, "input: {input:?}");
    }
}

#[test]
fn applied_rules_are_reported_by_category() {
    let normalized = engine().normalize("Dr. Sharma ne ₹500 diye");

    assert_eq!(normalized.text, "doctor Sharma ne 500 rupaye diye");
    assert_eq!(normalized.rules_applied, ["abbreviations", "currency"]);
}

#[test]
fn ordinary_words_are_left_alone() {
    let engine = engine();
    let untouched = [
        "mera naam h k sharma hai",
        "to err is human",
        "the horse eats hay",
        "drive to the address",
        "I like this etching",
    ];

    for text in untouched {
        assert_eq!(engine.normalize(text).text, text);
    }
}

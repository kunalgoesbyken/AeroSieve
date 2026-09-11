use aerosieve_lexical::RuleEngine;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::path::PathBuf;

const SHORT: &str = "yeh \u{20B9}500 hai aur 5 laakh rupaye ka hisaab hai";

/// No rule keyword appears here, so the Aho-Corasick pre-filter should reject
/// every keyword-gated rule and only `always_check` rules run.
const NO_MATCH: &str = "the quick brown fox jumped over twelve sleeping wolves \
     beside a quiet riverbank while nobody watched from the wooden bridge";

fn long_paragraph() -> String {
    let sentences = [
        "yeh \u{20B9}500 hai aur 5 laakh rupaye ka hisaab hai",
        "mujhe kal jana hai, prof. sharma ne bola tha",
        "govt. dept. ne 2 crore ka budget diya 05/09/2026 ko",
        "main kaam kar raha hu abhi, bahut thoda time bacha hai",
        "<b>nahin</b> yaar, Rs. 1200 mein kuch nahee milta &nbsp; achchha",
        "umm 15th march ko meeting hai, etc. etc. vs. dusra plan",
    ];
    let mut s = String::new();
    for _ in 0..40 {
        for sentence in sentences {
            s.push_str(sentence);
            s.push_str(". ");
        }
    }
    s
}

fn engine() -> RuleEngine {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rules/default.yaml");
    RuleEngine::from_yaml_file(&path).expect("default rules must load")
}

fn bench_normalize(c: &mut Criterion) {
    let engine = engine();
    let long = long_paragraph();

    let mut group = c.benchmark_group("lexical_normalize");
    group.bench_function("short_hinglish", |b| {
        b.iter(|| engine.normalize(black_box(SHORT)))
    });
    group.bench_function("long_paragraph", |b| {
        b.iter(|| engine.normalize(black_box(long.as_str())))
    });
    group.bench_function("no_match_prefilter", |b| {
        b.iter(|| engine.normalize(black_box(NO_MATCH)))
    });
    group.finish();
}

fn bench_compile(c: &mut Criterion) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rules/default.yaml");
    let yaml = std::fs::read_to_string(&path).expect("default rules must be readable");
    c.bench_function("lexical_compile_default_rules", |b| {
        b.iter(|| RuleEngine::from_yaml(black_box(yaml.as_str())).unwrap())
    });
}

criterion_group!(benches, bench_normalize, bench_compile);
criterion_main!(benches);

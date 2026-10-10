//! Normalization and compilation cost of the shipped Hinglish ruleset. Run
//! with `cargo bench -p aerosieve-lexical`.

use std::hint::black_box;

use aerosieve_lexical::RuleEngine;
use criterion::{Criterion, criterion_group, criterion_main};

const RULES: &str = include_str!("../rules/hinglish.yaml");

const SENTENCES: [&str; 6] = [
    "yeh \u{20B9}500 hai aur 5 laakh rupaye ka hisaab hai",
    "mujhe kal jana hai, prof. sharma ne bola tha",
    "govt. dept. ne 2 crore ka budget diya 05/09/2026 ko",
    "main kaam kar raha hu abhi, bahut thoda time bacha hai",
    "<b>nahin</b> yaar, Rs. 1200 mein kuch nahee milta &nbsp; achchha",
    "umm 15th march ko meeting hai, etc. etc. vs. dusra plan",
];

const NO_MATCH: &str = "the quick brown fox jumped over twelve sleeping wolves \
     beside a quiet riverbank while nobody watched from the wooden bridge";

fn bench_normalize(c: &mut Criterion) {
    let engine = RuleEngine::from_yaml(RULES).unwrap();
    let paragraph = SENTENCES.join(". ").repeat(40);

    let mut group = c.benchmark_group("normalize");
    group.bench_function("sentence", |b| {
        b.iter(|| engine.normalize(black_box(SENTENCES[0])))
    });
    group.bench_function("paragraph", |b| {
        b.iter(|| engine.normalize(black_box(&paragraph)))
    });
    group.bench_function("no_match", |b| {
        b.iter(|| engine.normalize(black_box(NO_MATCH)))
    });
    group.finish();
}

fn bench_compile(c: &mut Criterion) {
    c.bench_function("compile_ruleset", |b| {
        b.iter(|| RuleEngine::from_yaml(black_box(RULES)).unwrap())
    });
}

criterion_group!(benches, bench_normalize, bench_compile);
criterion_main!(benches);

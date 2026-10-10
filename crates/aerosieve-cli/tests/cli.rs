//! Runs the built `aerosieve` binary end to end.

use std::path::Path;
use std::process::{Command, Output};

use aerosieve_fixtures::{SAMPLE_RATE, glottal_speech, samples, silence, tone};
use tempfile::TempDir;

fn aerosieve(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aerosieve"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn write_wav(path: &Path, audio: &[f32]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for &sample in audio {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
}

/// One speech recording with a transcript beside it, and one pure tone.
fn corpus() -> TempDir {
    let dir = TempDir::new().unwrap();
    let speech = [
        silence(samples(0.25)),
        glottal_speech(samples(4.0)),
        silence(samples(0.25)),
    ]
    .concat();
    write_wav(&dir.path().join("speech.wav"), &speech);
    write_wav(
        &dir.path().join("tone.wav"),
        &tone(SAMPLE_RATE, 440.0, 0.3, samples(4.0)),
    );
    std::fs::write(dir.path().join("speech.txt"), "yeh  \u{20B9}500 hai").unwrap();
    dir
}

#[test]
fn curate_writes_accepted_clips_and_a_nemo_manifest_can_be_exported() {
    let corpus = corpus();
    let out = TempDir::new().unwrap();
    let out_dir = out.path().join("run");
    let rules = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../aerosieve-lexical/rules/hinglish.yaml"
    );

    let curated = aerosieve(&[
        "curate",
        corpus.path().to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--rules",
        rules,
    ]);

    assert!(curated.status.success(), "{curated:?}");
    assert!(stdout(&curated).contains("1 of 2 clips accepted"));
    assert!(stdout(&curated).contains("rejected as tonal"));
    assert!(out_dir.join("shard-000000.tar").exists());
    let manifest = std::fs::read_to_string(out_dir.join("manifest.jsonl")).unwrap();
    assert!(manifest.contains("yeh 500 rupaye hai"));

    let exported = aerosieve(&["nemo-manifest", out_dir.to_str().unwrap()]);

    assert!(exported.status.success(), "{exported:?}");
    let nemo = std::fs::read_to_string(out_dir.join("nemo_manifest.jsonl")).unwrap();
    let entry: serde_json::Value = serde_json::from_str(nemo.trim()).unwrap();
    assert_eq!(entry["audio_filepath"], "000000000.wav");
    assert_eq!(entry["text"], "yeh 500 rupaye hai");
    assert_eq!(entry["shard_id"], 0);
    assert!(entry["duration"].as_f64().unwrap() > 3.0);
}

#[test]
fn inspect_reports_verdicts_and_honours_a_policy_file() {
    let corpus = corpus();
    let policy = corpus.path().join("policy.json");
    std::fs::write(&policy, r#"{"min_snr_db": 90.0}"#).unwrap();
    let corpus_dir = corpus.path().to_str().unwrap();

    let default = aerosieve(&["inspect", corpus_dir]);
    let strict = aerosieve(&[
        "inspect",
        corpus_dir,
        "--summary-only",
        "--policy",
        policy.to_str().unwrap(),
    ]);

    assert!(default.status.success(), "{default:?}");
    assert!(stdout(&default).contains("speech.wav"));
    assert!(stdout(&default).contains("1 of 2 clips accepted"));
    assert!(strict.status.success(), "{strict:?}");
    assert!(!stdout(&strict).contains("speech.wav"));
    assert!(stdout(&strict).contains("0 of 2 clips accepted"));
    assert!(stdout(&strict).contains("rejected as low_snr"));
}

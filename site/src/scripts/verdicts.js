const snr = (c) => `${c.wada_snr_db.toFixed(1)} dB SNR`;
const level = (c) => `${c.rms_dbfs.toFixed(1)} dBFS`;

const VERDICTS = {
  accept: { label: "kept", tone: "accept", reason: snr },
  low_snr: { label: "low_snr", tone: "warn", reason: snr },
  clipping: { label: "clipping", tone: "danger", reason: (c) => `${(c.clip_ratio * 100).toFixed(1)}% clipped` },
  tonal: { label: "tonal", tone: "violet", reason: (c) => `flatness ${c.spectral_flatness.toFixed(3)}` },
  too_quiet: { label: "too_quiet", tone: "muted", reason: level },
  too_loud: { label: "too_loud", tone: "danger", reason: level },
  silence: { label: "silence", tone: "muted", reason: () => "no signal" },
};

const UNKNOWN = { label: "rejected", tone: "muted", reason: () => "" };

export const verdictOf = (clip) => VERDICTS[clip.verdict] ?? { ...UNKNOWN, label: clip.verdict };

export const isKept = (clip) => clip.verdict === "accept";

export const describe = (clip) => {
  const verdict = verdictOf(clip);
  return { ...verdict, detail: verdict.reason(clip) };
};

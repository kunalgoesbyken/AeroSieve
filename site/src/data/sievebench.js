import corpora from "../../public/data/sievebench-index.json";

const files = import.meta.glob("../../public/data/sievebench-*.json", { eager: true, import: "default" });

export const REPORTS = corpora.map((id) => files[`../../public/data/sievebench-${id}.json`]);

const sum = (items, value) => items.reduce((total, item) => total + value(item), 0);

const defectFamilies = (report) => report.families.filter((family) => family.family !== "control");

export const keptShare = (report, id) => {
  const condition = report.conditions.find((entry) => entry.id === id);
  return condition?.clips ? condition.kept / condition.clips : 0;
};

export const rightReasonShare = (report) => {
  const families = defectFamilies(report);
  const expected = sum(families, (family) => family.should_reject);
  return expected ? sum(families, (family) => family.right_reason) / expected : 0;
};

const recordings = sum(REPORTS, (report) => report.utterances);
const caught = sum(REPORTS, (report) => report.overall.caught);

export const TOTALS = {
  recordings,
  clips: sum(REPORTS, (report) => report.utterances * report.conditions.length),
  unsegmented: sum(REPORTS, (report) => report.overall.unsegmented),
  caught,
  expected: sum(REPORTS, (report) => report.overall.should_reject),
  misnamed: caught - sum(REPORTS, (report) => sum(defectFamilies(report), (family) => family.right_reason)),
};

export const count = (value) => value.toLocaleString("en");

export const LINK_LABELS = {
  github: "Source on GitHub",
  crates: "crates.io",
  docsRs: "API reference on docs.rs",
  changelog: "Changelog",
};

export const DOCS = [
  {
    slug: "index",
    title: "Get started",
    group: "Start",
    description: "Install AeroSieve and curate your first recordings in three commands.",
    links: ["github", "crates", "changelog"],
  },
  {
    slug: "cli",
    title: "Command line",
    group: "Use",
    description: "Every command and flag of the aerosieve tool, and the files a run writes.",
    links: ["github"],
  },
  {
    slug: "python",
    title: "Python",
    group: "Use",
    description: "Inspect recordings and write shards from Python with aerosieve.inspect and aerosieve.curate.",
    links: ["github"],
  },
  {
    slug: "rust",
    title: "Rust library",
    group: "Use",
    description: "Pipeline, Inspector and LiveInspector: the same code the CLI runs, as a crate.",
    links: ["docsRs", "crates"],
  },
  {
    slug: "streaming",
    title: "Streaming server",
    group: "Use",
    description: "Judge live audio over a WebSocket with aerosieve-serve, one verdict per utterance.",
    links: ["github"],
  },
  {
    slug: "filtering",
    title: "How clips are judged",
    group: "Reference",
    description: "The four measurements, every threshold with its default and reject reason, and how to tune them.",
    links: ["github"],
  },
];

export const GROUPS = [...new Set(DOCS.map((doc) => doc.group))];

export const docHref = (slug) => (slug === "index" ? "/docs" : `/docs/${slug}`);

export const docFromPath = (pathname) => {
  const last = pathname.split("/").filter(Boolean).pop();
  return DOCS.find((doc) => doc.slug === (last === "docs" ? "index" : last));
};

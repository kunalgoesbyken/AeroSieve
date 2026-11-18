import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://aerosieve.kunalrc.xyz",
  trailingSlash: "ignore",
  markdown: { shikiConfig: { theme: "github-light" } },
});

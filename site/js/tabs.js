export function mountTabs(root) {
  root.querySelectorAll("[data-tabs]").forEach((group) => {
    const tabs = [...group.querySelectorAll("[role=tab]")];
    const select = (chosen) =>
      tabs.forEach((tab) => {
        const selected = tab === chosen;
        tab.setAttribute("aria-selected", String(selected));
        document.getElementById(tab.getAttribute("aria-controls")).hidden = !selected;
      });
    tabs.forEach((tab) => tab.addEventListener("click", () => select(tab)));
  });
}

export function mountTabs(root) {
  root.querySelectorAll("[data-tabs]").forEach((group) => {
    const list = group.querySelector("[role=tablist]");
    const indicator = group.querySelector("[data-tab-indicator]");
    const tabs = [...list.querySelectorAll("[role=tab]")];
    const current = () => tabs.find((tab) => tab.getAttribute("aria-selected") === "true");
    const place = (tab) => {
      indicator.style.setProperty("--x", `${tab.offsetLeft}px`);
      indicator.style.setProperty("--w", `${tab.offsetWidth}px`);
    };
    const select = (chosen) => {
      tabs.forEach((tab) => {
        const selected = tab === chosen;
        tab.setAttribute("aria-selected", String(selected));
        document.getElementById(tab.getAttribute("aria-controls")).hidden = !selected;
      });
      place(chosen);
    };
    tabs.forEach((tab) => tab.addEventListener("click", () => select(tab)));
    new ResizeObserver(() => place(current())).observe(list);
  });
}

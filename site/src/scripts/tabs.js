const STEPS = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 };

export function mountTabs(root) {
  root.querySelectorAll("[data-tabs]").forEach((group) => {
    const tabs = [...group.querySelectorAll("[role=tab]")];
    const select = (chosen) => {
      tabs.forEach((tab) => {
        const selected = tab === chosen;
        tab.setAttribute("aria-selected", String(selected));
        tab.tabIndex = selected ? 0 : -1;
        document.getElementById(tab.getAttribute("aria-controls")).hidden = !selected;
      });
    };
    const move = (from, offset) => {
      const next = tabs[(tabs.indexOf(from) + offset + tabs.length) % tabs.length];
      select(next);
      next.focus();
    };
    tabs.forEach((tab) => {
      tab.addEventListener("click", () => select(tab));
      tab.addEventListener("keydown", (event) => {
        if (event.key === "Home") move(tabs[0], 0);
        else if (event.key === "End") move(tabs.at(-1), 0);
        else if (event.key in STEPS) move(tab, STEPS[event.key]);
        else return;
        event.preventDefault();
      });
    });
    select(tabs.find((tab) => tab.getAttribute("aria-selected") === "true") ?? tabs[0]);
  });
}

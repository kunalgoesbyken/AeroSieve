import { element } from "./dom.js";

const CONFIRM_MS = 1600;

const copyLabels = () => [element("span", { className: "copy", text: "Copy" }), element("span", { className: "copied", text: "Copied" })];

export function addCodeCopyButtons(root) {
  root.querySelectorAll("[data-code-copy] pre").forEach((pre) => {
    const button = element("button", { type: "button", className: "copy-command", "data-copy": pre.textContent.trimEnd() }, copyLabels());
    const block = element("div", { className: "code-block" }, [element("div", { className: "code-bar" }, [button])]);
    pre.replaceWith(block);
    block.append(pre);
  });
}

export function mountCopyButtons(root) {
  root.querySelectorAll("[data-copy]").forEach((button) => {
    button.addEventListener("click", async () => {
      await navigator.clipboard.writeText(button.dataset.copy);
      button.classList.add("is-copied");
      setTimeout(() => button.classList.remove("is-copied"), CONFIRM_MS);
    });
  });
}

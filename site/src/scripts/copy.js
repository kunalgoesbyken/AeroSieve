const CONFIRM_MS = 1600;

export function addCodeCopyButtons(root) {
  root.querySelectorAll("[data-code-copy] pre").forEach((pre) => {
    const block = document.createElement("div");
    const bar = document.createElement("div");
    const button = document.createElement("button");
    block.className = "code-block";
    bar.className = "code-bar";
    button.type = "button";
    button.className = "copy-command";
    button.dataset.copy = pre.textContent.trimEnd();
    button.innerHTML = '<span class="copy">Copy</span><span class="copied">Copied</span>';
    pre.replaceWith(block);
    bar.append(button);
    block.append(bar, pre);
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

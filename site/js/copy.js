const CONFIRM_MS = 1600;

export function mountCopyButtons(root) {
  root.querySelectorAll("[data-copy]").forEach((button) => {
    button.addEventListener("click", async () => {
      await navigator.clipboard.writeText(button.dataset.copy);
      button.classList.add("is-copied");
      setTimeout(() => button.classList.remove("is-copied"), CONFIRM_MS);
    });
  });
}

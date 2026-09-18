// Close a modal on a backdrop click — but only a real one. A drag that starts
// in the panel (e.g. selecting text in a field) and is released on the backdrop
// still fires `click` on the backdrop, the common ancestor; that must not
// dismiss the modal and throw away what was typed (#47). So both the press and
// the release have to land on the backdrop itself.
export function backdropClose(node: HTMLElement, close: () => void) {
  let pressed = false;
  const down = (e: MouseEvent) => (pressed = e.target === node);
  const click = (e: MouseEvent) => {
    if (pressed && e.target === node) close();
    pressed = false;
  };
  node.addEventListener("mousedown", down);
  node.addEventListener("click", click);
  return {
    update: (next: () => void) => (close = next),
    destroy() {
      node.removeEventListener("mousedown", down);
      node.removeEventListener("click", click);
    },
  };
}

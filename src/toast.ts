// Typewriter speech bubble for the latest notification, mirroring
// SumVox's Swift toast.

const CHAR_MS = 30;
const HOLD_MS = 3000;

let gen = 0;

export function toast(text: string) {
  const el = document.getElementById("toast")!;
  const my = ++gen;
  el.textContent = "";
  el.classList.add("show");
  let i = 0;
  const type = () => {
    if (my !== gen) return;
    if (i < text.length) {
      el.textContent = text.slice(0, ++i);
      setTimeout(type, CHAR_MS);
    } else {
      setTimeout(() => {
        if (my === gen) el.classList.remove("show");
      }, HOLD_MS);
    }
  };
  type();
}

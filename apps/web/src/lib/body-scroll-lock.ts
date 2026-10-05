// Restore through CSSOM: setting the whole style attribute is blocked by the app CSP.
const locks = new WeakMap<Document, {count:number;overflow:string;priority:string}>();
export function lockBodyScroll(document: Document): () => void {
  const current = locks.get(document);
  if (current) current.count++;
  else {
    locks.set(document, {count:1,overflow:document.body.style.getPropertyValue("overflow"),priority:document.body.style.getPropertyPriority("overflow")});
    document.body.style.setProperty("overflow", "hidden");
  }
  return () => {
    const state=locks.get(document);
    if (!state || --state.count) return;
    if (state.overflow) document.body.style.setProperty("overflow",state.overflow,state.priority);
    else document.body.style.removeProperty("overflow");
    locks.delete(document);
  };
}

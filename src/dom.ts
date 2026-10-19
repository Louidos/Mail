/**
 * Kleine Hilfen zum Aufbau der Oberfläche.
 *
 * Texte werden immer als Text gesetzt (nie als HTML), damit Inhalte von
 * Servern oder Mails nicht als Code ausgeführt werden können.
 */

type Kind = Node | string | null | undefined | false;

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  eigenschaften: Record<string, string | boolean | undefined> = {},
  ...kinder: Kind[]
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  for (const [name, wert] of Object.entries(eigenschaften)) {
    if (wert === undefined || wert === false) continue;
    if (name.startsWith("on")) {
      throw new Error("Ereignisse bitte mit addEventListener setzen");
    }
    element.setAttribute(name, wert === true ? "" : wert);
  }
  for (const kind of kinder) {
    if (kind === null || kind === undefined || kind === false) continue;
    element.append(kind);
  }
  return element;
}

/** Sucht ein Element und meldet einen klaren Fehler, falls es fehlt. */
export function finde<T extends Element>(wurzel: ParentNode, selektor: string): T {
  const element = wurzel.querySelector<T>(selektor);
  if (!element) {
    throw new Error(`Element ${selektor} fehlt`);
  }
  return element;
}

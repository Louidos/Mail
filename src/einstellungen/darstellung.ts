/**
 * Darstellung hell oder dunkel (#47).
 *
 * Die Farben stehen in `src/styles.css`. Ohne `data-darstellung` am `<html>`-Element folgt
 * SicherMail der Systemeinstellung (`prefers-color-scheme`), auch wenn sie sich im Betrieb
 * ändert. Beim Start setzt das Backend die gespeicherte Wahl schon vor dem ersten Zeichnen
 * (siehe `src-tauri/src/lib.rs`), damit nichts hell aufblitzt.
 */
import { api } from "../api";

export type Darstellung = "automatisch" | "hell" | "dunkel";

/** Auswahl im Einstellungsdialog. */
export const DARSTELLUNGEN: readonly { wert: Darstellung; text: string }[] = [
  { wert: "automatisch", text: "Automatisch (wie das System)" },
  { wert: "hell", text: "Hell" },
  { wert: "dunkel", text: "Dunkel" },
];

const SYSTEM_DUNKEL = "(prefers-color-scheme: dark)";

let gewaehlt: Darstellung = "automatisch";
let beobachtet = false;

function systemIstDunkel(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia(SYSTEM_DUNKEL).matches;
}

/** Tatsächlich sichtbare Darstellung. */
export function aktiveDarstellung(darstellung: Darstellung = gewaehlt): "hell" | "dunkel" {
  if (darstellung !== "automatisch") return darstellung;
  return systemIstDunkel() ? "dunkel" : "hell";
}

/**
 * Wendet die Darstellung sofort an. `data-darstellung` erzwingt „Hell“ oder „Dunkel“;
 * `data-aktiv` zeigt, was gerade sichtbar ist (auch bei „Automatisch“).
 */
export function darstellungAnwenden(darstellung: Darstellung, wurzel: HTMLElement = document.documentElement): void {
  gewaehlt = darstellung;
  if (darstellung === "automatisch") {
    delete wurzel.dataset.darstellung;
  } else {
    wurzel.dataset.darstellung = darstellung;
  }
  wurzel.dataset.aktiv = aktiveDarstellung(darstellung);
}

/** Zuletzt angewendete Wahl. */
export function gewaehlteDarstellung(): Darstellung {
  return gewaehlt;
}

/** Folgt einem Wechsel der Systemeinstellung, solange „Automatisch“ gewählt ist. */
export function systemBeobachten(): void {
  if (beobachtet || typeof window.matchMedia !== "function") return;
  beobachtet = true;
  window.matchMedia(SYSTEM_DUNKEL).addEventListener("change", () => {
    if (gewaehlt === "automatisch") darstellungAnwenden("automatisch");
  });
}

/** Beim Start: gesetzte Wahl übernehmen, gespeicherte Einstellung laden und das System beobachten. */
export async function darstellungStarten(wurzel: HTMLElement = document.documentElement): Promise<void> {
  const vorab = wurzel.dataset.darstellung;
  darstellungAnwenden(vorab === "hell" || vorab === "dunkel" ? vorab : "automatisch", wurzel);
  systemBeobachten();
  try {
    darstellungAnwenden((await api.einstellungenLaden()).darstellung, wurzel);
  } catch {
    // Ausserhalb von Tauri (z. B. in Tests) bleibt es bei der Systemeinstellung.
  }
}

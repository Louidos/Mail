/**
 * Suche in Mails (#49): Suchleiste über der Mailliste und Hervorheben der Suchbegriffe.
 *
 * Gesucht wird zuerst nur lokal im verschlüsselten Speicher. Erst ein Klick auf
 * „Auf dem Server suchen“ schickt den Suchbegriff an den eigenen IMAP-Server.
 */
import type { SuchFeld } from "../api";
import { el } from "../dom";

/** So lange wird nach der letzten Eingabe gewartet, bevor gesucht wird. */
export const SUCH_VERZOEGERUNG = 300;

/** So viele Treffer werden pro Seite geladen. */
export const SUCH_SEITE = 200;

/** Lokal: im gewählten Ordner oder in allen Ordnern des Kontos. */
export type SuchBereich = "ordner" | "konto";

export const FELDER: [SuchFeld, string][] = [
  ["alle", "Alle Felder"],
  ["absender", "Absender"],
  ["betreff", "Betreff"],
  ["inhalt", "Inhalt"],
];

export const BEREICHE: [SuchBereich, string][] = [
  ["ordner", "Dieser Ordner"],
  ["konto", "Alle Ordner des Kontos"],
];

/** Wörter des Suchbegriffs (getrennt an Leerraum und Anführungszeichen, nur mit Buchstaben oder Ziffern). */
export function suchwoerter(begriff: string): string[] {
  return begriff
    .split(/[\s"]+/u)
    .filter((wort) => /[\p{L}\p{N}]/u.test(wort))
    .slice(0, 10);
}

/** Vereinfachte Form für den Vergleich: klein, ohne Akzente (ü wie u, wie die lokale Suche). */
function einfach(zeichen: string): string {
  return zeichen.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

/**
 * Text mit hervorgehobenen Suchwörtern: Treffer stehen in `<mark>`. Alles wird als Text gesetzt,
 * nie als HTML.
 */
export function hervorheben(text: string, begriff: string): Node[] {
  const woerter = suchwoerter(begriff).map((w) => [...w].map(einfach).join("")).filter((w) => w.length > 0);
  if (woerter.length === 0 || text.length === 0) return [document.createTextNode(text)];
  // Vereinfachte Fassung mit Zuordnung zu den Zeichen des Originals.
  const zeichen = [...text];
  let vereinfacht = "";
  const herkunft: number[] = [];
  zeichen.forEach((z, i) => {
    const e = einfach(z);
    vereinfacht += e;
    for (let n = 0; n < e.length; n++) herkunft.push(i);
  });
  const markiert = new Array<boolean>(zeichen.length).fill(false);
  for (const wort of woerter) {
    for (let pos = vereinfacht.indexOf(wort); pos !== -1; pos = vereinfacht.indexOf(wort, pos + 1)) {
      for (let n = pos; n < pos + wort.length; n++) markiert[herkunft[n]] = true;
    }
  }
  const teile: Node[] = [];
  let i = 0;
  while (i < zeichen.length) {
    const an = markiert[i];
    let j = i;
    while (j < zeichen.length && markiert[j] === an) j++;
    const stueck = zeichen.slice(i, j).join("");
    teile.push(an ? el("mark", {}, stueck) : document.createTextNode(stueck));
    i = j;
  }
  return teile;
}

export interface SuchleisteAktionen {
  /** Lokal suchen (nach der Eingabeverzögerung, bei Enter sofort). */
  suchen: () => void;
  /** Ausdrücklich auf dem Server suchen. */
  serverSuchen: () => void;
  /** Suche beenden und zur vorherigen Ansicht zurück. */
  beenden: () => void;
}

/** Die Suchleiste. Sie bleibt bestehen, damit Eingabe und Fokus beim Neuzeichnen erhalten bleiben. */
export class Suchleiste {
  readonly element: HTMLFormElement;
  readonly feld: HTMLInputElement;
  readonly feldWahl: HTMLSelectElement;
  readonly bereichWahl: HTMLSelectElement;
  readonly serverKnopf: HTMLButtonElement;
  readonly beendenKnopf: HTMLButtonElement;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly aktionen: SuchleisteAktionen) {
    this.feld = el("input", {
      type: "search",
      name: "suche",
      class: "suchfeld",
      placeholder: "Mails durchsuchen (Ctrl+F)",
      "aria-label": "Mails durchsuchen",
      "aria-keyshortcuts": "Control+F Meta+F",
      autocomplete: "off",
      spellcheck: "false",
    });
    this.feldWahl = el(
      "select",
      { name: "suchfeld", class: "klein", "aria-label": "Suchen in" },
      ...FELDER.map(([wert, text]) => el("option", { value: wert }, text)),
    );
    this.bereichWahl = el(
      "select",
      { name: "suchbereich", class: "klein", "aria-label": "Suchbereich" },
      ...BEREICHE.map(([wert, text]) => el("option", { value: wert }, text)),
    );
    this.serverKnopf = el(
      "button",
      {
        type: "button",
        class: "klein",
        disabled: true,
        title: "Schickt den Suchbegriff an deinen eigenen Mailserver. Findet auch Mails, die noch nicht auf diesem Gerät liegen.",
      },
      "Auf dem Server suchen",
    );
    this.beendenKnopf = el("button", { type: "button", class: "klein", hidden: true }, "Suche beenden");
    this.element = el(
      "form",
      { class: "suchleiste", role: "search" },
      this.feld,
      el("div", { class: "such-optionen" }, this.feldWahl, this.bereichWahl, this.serverKnopf, this.beendenKnopf),
    );

    this.element.addEventListener("submit", (ereignis) => {
      ereignis.preventDefault();
      this.sofortSuchen();
    });
    this.feld.addEventListener("input", () => {
      this.knoepfeAktualisieren();
      if (this.timer) clearTimeout(this.timer);
      this.timer = setTimeout(() => {
        this.timer = null;
        this.aktionen.suchen();
      }, SUCH_VERZOEGERUNG);
    });
    this.feld.addEventListener("keydown", (ereignis) => {
      if (ereignis.key === "Escape") {
        ereignis.preventDefault();
        this.aktionen.beenden();
      }
    });
    this.feldWahl.addEventListener("change", () => this.sofortSuchen());
    this.bereichWahl.addEventListener("change", () => this.sofortSuchen());
    this.serverKnopf.addEventListener("click", () => {
      this.abbrechen();
      this.aktionen.serverSuchen();
    });
    this.beendenKnopf.addEventListener("click", () => this.aktionen.beenden());
  }

  get begriff(): string {
    return this.feld.value.trim();
  }

  get suchFeld(): SuchFeld {
    return this.feldWahl.value as SuchFeld;
  }

  get bereich(): SuchBereich {
    return this.bereichWahl.value as SuchBereich;
  }

  /** Setzt den Fokus ins Suchfeld und markiert den bisherigen Begriff. */
  fokussieren(): void {
    this.feld.focus();
    this.feld.select();
  }

  /** Leert das Feld (ohne zu suchen). */
  leeren(): void {
    this.abbrechen();
    this.feld.value = "";
    this.knoepfeAktualisieren();
  }

  /** Zeigt „Suche beenden“, solange eine Suche aktiv ist. */
  aktivSetzen(aktiv: boolean): void {
    this.beendenKnopf.hidden = !aktiv;
    this.knoepfeAktualisieren();
  }

  private sofortSuchen(): void {
    this.abbrechen();
    this.knoepfeAktualisieren();
    this.aktionen.suchen();
  }

  private abbrechen(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private knoepfeAktualisieren(): void {
    this.serverKnopf.disabled = suchwoerter(this.begriff).length === 0;
  }
}

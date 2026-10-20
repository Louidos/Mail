/** Kennzeichnung der Konten im gemeinsamen Posteingang (#50). */
import type { Konto } from "../api";
import { el } from "../dom";

/** So viele Kontofarben gibt es (`--konto-1` … in styles.css); danach wiederholen sie sich. */
export const KONTOFARBEN = 6;

export interface KontoKennung {
  /** Ein bis zwei Buchstaben, z. B. „AM“ für „Anna Muster“. */
  kuerzel: string;
  /** Lesbarer Name: Kontoname, bei gleichen Namen die Adresse. */
  name: string;
  /** Nummer der Kontofarbe (1 bis {@link KONTOFARBEN}). */
  farbe: number;
}

/** Kürzel aus dem Namen (Anfangsbuchstaben der ersten zwei Wörter), sonst aus der Adresse. */
export function kuerzel(konto: Konto): string {
  const woerter = konto.name.split(/\s+/u).filter((w) => /[\p{L}\p{N}]/u.test(w));
  const quelle = woerter.length > 0 ? woerter.slice(0, 2) : [konto.email];
  return quelle
    .map((w) => [...w].find((z) => /[\p{L}\p{N}]/u.test(z)) ?? "")
    .join("")
    .toUpperCase();
}

/** Kennung eines Kontos; Farbe nach der Reihenfolge der Konten. */
export function kontoKennung(konto: Konto, konten: Konto[]): KontoKennung {
  const nummer = Math.max(0, konten.findIndex((k) => k.id === konto.id));
  const gleicherName = konten.some((k) => k.id !== konto.id && k.name === konto.name);
  return {
    kuerzel: kuerzel(konto) || "?",
    name: gleicherName || !konto.name.trim() ? konto.email : konto.name,
    farbe: (nummer % KONTOFARBEN) + 1,
  };
}

/** Kontozeichen für eine Zeile der Mailliste: Kürzel mit Farbe und der Name als Text – nie nur Farbe. */
export function kontoMarke(konto: Konto | undefined, konten: Konto[]): HTMLElement {
  if (!konto) return el("span", { class: "mail-konto" }, "Unbekanntes Konto");
  const kennung = kontoKennung(konto, konten);
  return el(
    "span",
    { class: "mail-konto", title: `Konto: ${konto.name} <${konto.email}>` },
    el("span", { class: "konto-kuerzel", "data-farbe": String(kennung.farbe), "aria-hidden": "true" }, kennung.kuerzel),
    el("span", { class: "unsichtbar" }, "Konto: "),
    kennung.name,
  );
}

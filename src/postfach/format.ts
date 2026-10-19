/** Formatierung für die Mailliste. */

const UHRZEIT = new Intl.DateTimeFormat("de-CH", { hour: "2-digit", minute: "2-digit" });
const TAG_MONAT = new Intl.DateTimeFormat("de-CH", { day: "numeric", month: "short" });
const VOLL = new Intl.DateTimeFormat("de-CH", { day: "2-digit", month: "2-digit", year: "numeric" });
const LANG = new Intl.DateTimeFormat("de-CH", { dateStyle: "full", timeStyle: "short" });

/** Kurzes Datum für die Liste: heute nur Uhrzeit, dieses Jahr Tag und Monat, sonst volles Datum. */
export function datumKurz(sekunden: number | null, jetzt: Date = new Date()): string {
  if (sekunden === null) return "";
  const datum = new Date(sekunden * 1000);
  if (datum.toDateString() === jetzt.toDateString()) return UHRZEIT.format(datum);
  if (datum.getFullYear() === jetzt.getFullYear()) return TAG_MONAT.format(datum);
  return VOLL.format(datum);
}

/** Ausführliches Datum für die Leseansicht. */
export function datumLang(sekunden: number | null): string {
  return sekunden === null ? "unbekannt" : LANG.format(new Date(sekunden * 1000));
}

/** Lesbare Grösse, z. B. „12 KB“. */
export function groesse(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1).replace(".", ",")} MB`;
}

/** Einrückungstiefe eines Ordners anhand seines Trennzeichens. */
export function ordnerTiefe(name: string, trenner: string | null): number {
  if (!trenner) return 0;
  return name.split(trenner).length - 1;
}

/** Letzter Teil eines Ordnernamens (ohne übergeordnete Ordner). */
export function ordnerKurzname(anzeigename: string, trenner: string | null): string {
  if (!trenner) return anzeigename;
  const teile = anzeigename.split(trenner);
  return teile[teile.length - 1] || anzeigename;
}

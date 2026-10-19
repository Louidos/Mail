/** Leseansicht: Kopfdaten und Aktionen einer geöffneten Mail. */
import type { MailKopf, Ordner } from "../api";
import { el } from "../dom";
import { datumLang, groesse } from "./format";

export interface LeseAktionen {
  gelesenUmschalten: (mail: MailKopf) => void;
  loeschen: (mail: MailKopf) => void;
  verschieben: (mail: MailKopf, ziel: string) => void;
}

/** Baut die Leseansicht für eine Mail. */
export function leseansicht(
  mail: MailKopf,
  aktuellerOrdner: string,
  ordner: Ordner[],
  aktionen: LeseAktionen,
): HTMLElement {
  const gelesenKnopf = el(
    "button",
    { type: "button", class: "klein" },
    mail.gelesen ? "Als ungelesen markieren" : "Als gelesen markieren",
  );
  gelesenKnopf.addEventListener("click", () => aktionen.gelesenUmschalten(mail));

  const loeschenKnopf = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
  loeschenKnopf.addEventListener("click", () => aktionen.loeschen(mail));

  const ziele = ordner.filter((o) => o.waehlbar && o.name !== aktuellerOrdner);
  const verschieben = el(
    "select",
    { class: "klein", "aria-label": "Verschieben nach" },
    el("option", { value: "" }, "Verschieben nach …"),
    ...ziele.map((o) => el("option", { value: o.name }, o.anzeigename)),
  );
  verschieben.addEventListener("change", () => {
    if (verschieben.value) aktionen.verschieben(mail, verschieben.value);
  });

  const absender =
    mail.absender && mail.absender !== mail.absender_adresse
      ? `${mail.absender} <${mail.absender_adresse}>`
      : mail.absender_adresse || mail.absender || "(unbekannt)";

  return el(
    "article",
    { class: "lesen" },
    el("div", { class: "werkzeuge" }, gelesenKnopf, verschieben, loeschenKnopf),
    el("h2", { class: "betreff" }, mail.betreff || "(kein Betreff)"),
    el(
      "dl",
      { class: "kopfdaten" },
      el("dt", {}, "Von"),
      el("dd", {}, absender),
      el("dt", {}, "Datum"),
      el("dd", {}, datumLang(mail.datum)),
      el("dt", {}, "Grösse"),
      el("dd", {}, groesse(mail.groesse)),
    ),
    el("div", { class: "inhalt-bereich", "data-uid": String(mail.uid) }),
  );
}

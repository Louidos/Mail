/** Leseansicht: Kopfdaten, Aktionen und sicher dargestellter Inhalt einer Mail. */
import type { Absenderpruefung, AbsenderStufe, Adresse, AufbereiteteMail, MailKopf, Ordner, PruefMethode } from "../api";
import { el } from "../dom";
import { datumLang, groesse } from "./format";
import { linksAbfangen, mailRahmenMitLinks, textMitLinks, type LinkKlick } from "./sicher";

export interface LeseAktionen {
  antworten: (mail: AufbereiteteMail, allen: boolean) => void;
  weiterleiten: (mail: AufbereiteteMail) => void;
  gelesenUmschalten: (mail: MailKopf) => void;
  loeschen: (mail: MailKopf) => void;
  verschieben: (mail: MailKopf, ziel: string) => void;
  nurTextUmschalten: (nurText: boolean) => void;
  externLaden: () => void;
  absenderErlauben: (adresse: string) => void;
  anhangSpeichern: (index: number) => void;
  linkKlick: (klick: LinkKlick) => void;
}

/** Zustand des Inhalts der geöffneten Mail. */
export type Inhalt =
  | { art: "laedt" }
  | { art: "fehler"; meldung: string }
  | { art: "fertig"; mail: AufbereiteteMail };

export interface LeseOptionen {
  nurText: boolean;
  /** Kurze Rückmeldung, z. B. nach dem Speichern eines Anhangs. */
  hinweis: string | null;
}

export function adresseText(adresse: Adresse): string {
  return adresse.name ? `${adresse.name} <${adresse.adresse}>` : adresse.adresse;
}

/** Text des Warnbalkens bei wahrscheinlich gefälschtem Absender. */
export const ABSENDER_WARNUNG =
  "Der Absender ist wahrscheinlich gefälscht. Antworte nicht, öffne keine Links und Anhänge, wenn du unsicher bist.";

/** Symbol und Text je Stufe – nie nur über die Farbe erkennbar. */
const STUFEN: Record<AbsenderStufe, { symbol: string; text: string }> = {
  bestaetigt: { symbol: "✓", text: "Absender bestätigt" },
  nicht_geprueft: { symbol: "–", text: "Absender nicht geprüft" },
  warnung: { symbol: "⚠", text: "Absender wahrscheinlich gefälscht" },
};

function methodeText(name: string, methode: PruefMethode | null | undefined): string {
  if (!methode) return `${name}: kein Ergebnis`;
  const domain = methode.domain ? ` für ${methode.domain}` : "";
  const passend = methode.domain && !methode.passend ? " (passt nicht zur Absenderdomain)" : "";
  return `${name}: ${methode.ergebnis}${domain}${passend}`;
}

/**
 * Kleines Feld beim Absender mit dem Ergebnis der Absenderprüfung. Ein Klick klappt die
 * Einzelheiten auf (DKIM, SPF, DMARC und die prüfende Server-Kennung).
 */
export function absenderFeld(pruefung: Absenderpruefung): HTMLDetailsElement {
  const stufe = STUFEN[pruefung.stufe] ?? STUFEN.nicht_geprueft;
  const zeilen: string[] = [];
  if (pruefung.gefunden) {
    zeilen.push(
      ...(pruefung.dkim.length > 0
        ? pruefung.dkim.map((m) => methodeText("DKIM", m))
        : [methodeText("DKIM", null)]),
      methodeText("SPF", pruefung.spf),
      methodeText("DMARC", pruefung.dmarc),
      `Geprüft von: ${pruefung.kennung}`,
    );
  } else {
    zeilen.push(
      pruefung.kennung
        ? `Kein Prüfergebnis deines Servers (Kennung „${pruefung.kennung}“) gefunden.`
        : "Keine Server-Kennung für die Absenderprüfung eingestellt.",
      "Absenderadressen lassen sich fälschen.",
    );
  }
  if (pruefung.absender_domain) zeilen.push(`Absenderdomain: ${pruefung.absender_domain}`);
  return el(
    "details",
    { class: "absender-pruefung", "data-stufe": pruefung.stufe },
    el(
      "summary",
      { title: "Einzelheiten zur Absenderprüfung" },
      el("span", { class: "symbol", "aria-hidden": "true" }, stufe.symbol),
      stufe.text,
    ),
    el("ul", {}, ...zeilen.map((zeile) => el("li", {}, zeile))),
  );
}

function inhaltBereich(
  inhalt: Inhalt,
  optionen: LeseOptionen,
  aktionen: LeseAktionen,
): HTMLElement {
  if (inhalt.art === "laedt") return el("p", { class: "leise" }, "Mail wird geladen …");
  if (inhalt.art === "fehler") return el("p", { class: "fehler", role: "alert" }, inhalt.meldung);

  const { mail } = inhalt;
  const teile: Node[] = [];

  const absender = mail.von[0]?.adresse ?? "";
  // Bei wahrscheinlich gefälschtem Absender keine dauerhafte Freigabe anbieten (#45).
  const gefaelscht = mail.absender_pruefung?.stufe === "warnung";
  if (mail.html && !optionen.nurText && mail.extern_blockiert > 0 && !mail.extern_geladen) {
    const laden = el("button", { type: "button", class: "klein" }, "Externe Inhalte laden");
    laden.addEventListener("click", () => aktionen.externLaden());
    const immer = absender && !gefaelscht
      ? el(
          "button",
          {
            type: "button",
            class: "klein",
            title: "Absenderadressen lassen sich fälschen. Nur für Absender verwenden, denen du vertraust.",
          },
          `Für ${absender} immer erlauben`,
        )
      : null;
    immer?.addEventListener("click", () => aktionen.absenderErlauben(absender));
    teile.push(
      el(
        "div",
        { class: "extern-leiste", role: "note" },
        el(
          "span",
          {},
          `${mail.extern_blockiert} externe ${mail.extern_blockiert === 1 ? "Inhalt wurde" : "Inhalte wurden"} blockiert, um dich vor Tracking zu schützen.`,
        ),
        laden,
        immer,
      ),
    );
  } else if (mail.html && !optionen.nurText && mail.extern_geladen && mail.extern_blockiert > 0) {
    teile.push(
      el(
        "div",
        { class: "extern-leiste", role: "note" },
        `${mail.extern_blockiert} externe Inhalte konnten nicht geladen werden (geladen wird nur über verschlüsseltes HTTPS).`,
      ),
    );
  }

  if (mail.html && !optionen.nurText) {
    teile.push(mailRahmenMitLinks(mail.html, mail.betreff, aktionen.linkKlick));
  } else {
    const text = el("div", { class: "mailtext" });
    text.append(textMitLinks(mail.text));
    linksAbfangen(text, aktionen.linkKlick);
    teile.push(text);
  }

  if (mail.anhaenge.length > 0) {
    const liste = el("ul", { class: "anhaenge" });
    for (const anhang of mail.anhaenge) {
      const speichern = el("button", { type: "button", class: "klein" }, "Speichern unter …");
      speichern.addEventListener("click", () => aktionen.anhangSpeichern(anhang.index));
      liste.append(
        el(
          "li",
          {},
          el("span", { class: "anhang-name" }, anhang.name),
          el("span", { class: "leise" }, `${anhang.typ}, ${groesse(anhang.groesse)}`),
          speichern,
        ),
      );
    }
    teile.push(
      el(
        "section",
        { class: "anhang-bereich" },
        el("h3", {}, `Anhänge (${mail.anhaenge.length})`),
        el("p", { class: "leise" }, "Anhänge werden nie automatisch geöffnet. Prüfe Dateien vor dem Öffnen."),
        liste,
      ),
    );
  }
  if (optionen.hinweis) {
    teile.push(el("p", { class: "hinweis", role: "status" }, optionen.hinweis));
  }
  return el("div", { class: "inhalt-bereich" }, ...teile);
}

/** Baut die Leseansicht für eine Mail. */
export function leseansicht(
  kopf: MailKopf,
  aktuellerOrdner: string,
  ordner: Ordner[],
  inhalt: Inhalt,
  optionen: LeseOptionen,
  aktionen: LeseAktionen,
): HTMLElement {
  const gelesenKnopf = el(
    "button",
    { type: "button", class: "klein" },
    kopf.gelesen ? "Als ungelesen markieren" : "Als gelesen markieren",
  );
  gelesenKnopf.addEventListener("click", () => aktionen.gelesenUmschalten(kopf));

  const loeschenKnopf = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
  loeschenKnopf.addEventListener("click", () => aktionen.loeschen(kopf));

  const ziele = ordner.filter((o) => o.waehlbar && o.name !== aktuellerOrdner);
  const verschieben = el(
    "select",
    { class: "klein", "aria-label": "Verschieben nach" },
    el("option", { value: "" }, "Verschieben nach …"),
    ...ziele.map((o) => el("option", { value: o.name }, o.anzeigename)),
  );
  verschieben.addEventListener("change", () => {
    if (verschieben.value) aktionen.verschieben(kopf, verschieben.value);
  });

  const nurText = el("input", { type: "checkbox", checked: optionen.nurText });
  nurText.addEventListener("change", () => aktionen.nurTextUmschalten(nurText.checked));

  const mail = inhalt.art === "fertig" ? inhalt.mail : null;
  const antworten = el("button", { type: "button", class: "klein haupt", disabled: !mail }, "Antworten");
  const allenAntworten = el("button", { type: "button", class: "klein", disabled: !mail }, "Allen antworten");
  const weiterleiten = el("button", { type: "button", class: "klein", disabled: !mail }, "Weiterleiten");
  if (mail) {
    antworten.addEventListener("click", () => aktionen.antworten(mail, false));
    allenAntworten.addEventListener("click", () => aktionen.antworten(mail, true));
    weiterleiten.addEventListener("click", () => aktionen.weiterleiten(mail));
  }
  const von = mail?.von.length
    ? mail.von.map(adresseText).join(", ")
    : kopf.absender && kopf.absender !== kopf.absender_adresse
      ? `${kopf.absender} <${kopf.absender_adresse}>`
      : kopf.absender_adresse || kopf.absender || "(unbekannt)";

  const pruefung = mail?.absender_pruefung;
  const kopfdaten: Node[] = [
    el("dt", {}, "Von"),
    el("dd", { class: "von" }, el("span", {}, von), pruefung ? absenderFeld(pruefung) : null),
  ];
  if (mail?.an.length) kopfdaten.push(el("dt", {}, "An"), el("dd", {}, mail.an.map(adresseText).join(", ")));
  if (mail?.cc.length) kopfdaten.push(el("dt", {}, "Cc"), el("dd", {}, mail.cc.map(adresseText).join(", ")));
  kopfdaten.push(el("dt", {}, "Datum"), el("dd", {}, datumLang(mail?.datum ?? kopf.datum)));

  return el(
    "article",
    { class: "lesen" },
    el(
      "div",
      { class: "werkzeuge" },
      antworten,
      allenAntworten,
      weiterleiten,
      gelesenKnopf,
      verschieben,
      loeschenKnopf,
      el("label", { class: "umschalter" }, nurText, "Nur Text anzeigen"),
    ),
    pruefung?.stufe === "warnung"
      ? el(
          "div",
          { class: "absender-warnung", role: "alert" },
          el("span", { class: "symbol", "aria-hidden": "true" }, "⚠"),
          el("strong", {}, "Achtung: "),
          ABSENDER_WARNUNG,
        )
      : null,
    el("h2", { class: "betreff" }, mail?.betreff || kopf.betreff || "(kein Betreff)"),
    el("dl", { class: "kopfdaten" }, ...kopfdaten),
    inhaltBereich(inhalt, optionen, aktionen),
  );
}

/** Fenster „Mail verfassen“: neue Mail, Antwort, Weiterleitung oder geöffneter Entwurf. */
import { api, fehlertext, type Entwurf, type Konto, type SendeErgebnis } from "../api";
import { bestaetigen, modalSchliessen, modalZeigen } from "../dialoge";
import { el } from "../dom";
import { groesse } from "../postfach/format";
import { empfaengerFehlen } from "./zitat";

export interface VerfassenOptionen {
  konten: Konto[];
  entwurf: Entwurf;
  /** Nach erfolgreichem Senden. */
  gesendet?: (ergebnis: SendeErgebnis) => void;
  /** Nachdem Entwürfe gespeichert oder gelöscht wurden. */
  entwuerfeGeaendert?: () => void;
}

function feld(beschriftung: string, eingabe: HTMLElement): HTMLLabelElement {
  return el("label", { class: "feld" }, el("span", { class: "beschriftung" }, beschriftung), eingabe);
}

/** Öffnet das Verfassen-Fenster. */
export function verfassenOeffnen(optionen: VerfassenOptionen): HTMLDialogElement {
  const entwurf: Entwurf = { ...optionen.entwurf, anhaenge: [...optionen.entwurf.anhaenge] };

  const von = el(
    "select",
    { name: "konto" },
    ...optionen.konten.map((k) =>
      el("option", { value: k.id, selected: k.id === entwurf.konto }, `${k.name} <${k.email}>`),
    ),
  );
  const an = el("input", { name: "an", type: "text", value: entwurf.an, autocomplete: "off" });
  const cc = el("input", { name: "cc", type: "text", value: entwurf.cc, autocomplete: "off" });
  const bcc = el("input", { name: "bcc", type: "text", value: entwurf.bcc, autocomplete: "off" });
  const betreff = el("input", { name: "betreff", type: "text", value: entwurf.betreff });
  const text = el("textarea", { name: "text", rows: "16", spellcheck: "true" });
  text.value = entwurf.text;
  const html = el("input", { type: "checkbox", name: "html", checked: entwurf.html });
  const anhangListe = el("ul", { class: "anhaenge" });
  const status = el("p", { class: "status", role: "status", "aria-live": "polite" });

  const anhaengen = el("button", { type: "button", class: "klein" }, "Datei anhängen …");
  const verwerfen = el("button", { type: "button", class: "gefahr" }, "Verwerfen");
  const speichern = el("button", { type: "button" }, "Als Entwurf speichern");
  const senden = el("button", { type: "submit", class: "haupt" }, "Senden");

  const form = el(
    "form",
    { method: "dialog", novalidate: true },
    el("h2", {}, entwurf.antwort_auf ? "Antworten" : "Mail verfassen"),
    feld("Von", von),
    feld("An", an),
    el("div", { class: "zeile" }, feld("Cc", cc), feld("Bcc", bcc)),
    feld("Betreff", betreff),
    feld("Text", text),
    el("label", { class: "umschalter" }, html, "Zusätzlich als einfaches HTML senden"),
    el("div", { class: "anhang-bereich" }, el("h3", {}, "Anhänge"), anhangListe, anhaengen),
    status,
    el("div", { class: "knoepfe" }, verwerfen, speichern, senden),
  );
  const dialog = el("dialog", { class: "verfassen-dialog" }, form);

  const statusSetzen = (meldung: string, art: "info" | "erfolg" | "fehler") => {
    status.textContent = meldung;
    status.dataset.art = art;
  };
  const beschaeftigt = (aktiv: boolean) => {
    for (const knopf of [anhaengen, verwerfen, speichern, senden]) knopf.disabled = aktiv;
  };
  /** Aktuellen Stand aus den Feldern übernehmen. */
  const lesen = (): Entwurf => {
    entwurf.konto = von.value;
    entwurf.an = an.value;
    entwurf.cc = cc.value;
    entwurf.bcc = bcc.value;
    entwurf.betreff = betreff.value;
    entwurf.text = text.value;
    entwurf.html = html.checked;
    return entwurf;
  };
  const anhaengeZeichnen = () => {
    anhangListe.replaceChildren(
      ...entwurf.anhaenge.map((anhang) => {
        const weg = el("button", { type: "button", class: "klein" }, "Entfernen");
        weg.addEventListener("click", () => {
          entwurf.anhaenge = entwurf.anhaenge.filter((a) => a.token !== anhang.token);
          anhaengeZeichnen();
        });
        return el(
          "li",
          {},
          el("span", { class: "anhang-name" }, anhang.name),
          el("span", { class: "leise" }, groesse(anhang.groesse)),
          weg,
        );
      }),
    );
  };
  anhaengeZeichnen();

  anhaengen.addEventListener("click", async () => {
    try {
      entwurf.anhaenge.push(...(await api.anhangWaehlen()));
      anhaengeZeichnen();
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    }
  });

  /** Speichert den Entwurf und merkt sich seine ID (damit es keine Doppel gibt). */
  const entwurfSichern = async () => {
    const gespeichert = await api.entwurfSpeichern(lesen());
    entwurf.id = gespeichert.id;
    optionen.entwuerfeGeaendert?.();
  };

  speichern.addEventListener("click", async () => {
    beschaeftigt(true);
    try {
      await entwurfSichern();
      statusSetzen("Entwurf gespeichert.", "erfolg");
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    } finally {
      beschaeftigt(false);
    }
  });

  form.addEventListener("submit", async (ereignis) => {
    ereignis.preventDefault();
    const daten = lesen();
    if (empfaengerFehlen(daten)) {
      statusSetzen("Bitte mindestens einen Empfänger angeben.", "fehler");
      return;
    }
    if (!daten.betreff.trim()) {
      const ok = await bestaetigen({ titel: "Ohne Betreff senden?", text: "Die Mail hat keinen Betreff.", ja: "Trotzdem senden" });
      if (!ok) return;
    }
    beschaeftigt(true);
    statusSetzen("Wird gesendet …", "info");
    try {
      // Erst sichern: Geht beim Senden etwas schief, ist der Text bereits gespeichert.
      await entwurfSichern();
      const ergebnis = await api.mailSenden(entwurf);
      optionen.entwuerfeGeaendert?.();
      modalSchliessen(dialog);
      optionen.gesendet?.(ergebnis);
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    } finally {
      beschaeftigt(false);
    }
  });

  verwerfen.addEventListener("click", async () => {
    const daten = lesen();
    const hatInhalt = daten.text.trim() !== "" || daten.betreff.trim() !== "" || daten.anhaenge.length > 0;
    if (hatInhalt || daten.id) {
      const ok = await bestaetigen({
        titel: "Mail verwerfen?",
        text: daten.id ? "Der Text und der gespeicherte Entwurf werden gelöscht." : "Der Text geht verloren.",
        ja: "Verwerfen",
        warnung: true,
      });
      if (!ok) return;
    }
    if (daten.id) {
      try {
        await api.entwurfLoeschen(daten.id);
        optionen.entwuerfeGeaendert?.();
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
        return;
      }
    }
    modalSchliessen(dialog);
  });

  // Escape schliesst nicht: Es soll kein Text aus Versehen verloren gehen.
  dialog.addEventListener("cancel", (ereignis) => ereignis.preventDefault());

  modalZeigen(dialog);
  (entwurf.an ? text : an).focus();
  if (entwurf.an && entwurf.antwort_auf) text.setSelectionRange(0, 0);
  return dialog;
}

/** Fenster „Mail verfassen“: neue Mail, Antwort, Weiterleitung oder geöffneter Entwurf. */
import {
  api,
  fehlertext,
  type Entwurf,
  type Konto,
  type SendeErgebnis,
  type Verfahren,
  type VersandPruefung,
} from "../api";
import { bestaetigen, modalSchliessen, modalZeigen, passphraseAbfragen } from "../dialoge";
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

/** Anzeigenamen der Verfahren. */
export const VERFAHREN_NAME: Record<Verfahren, string> = { openpgp: "OpenPGP", smime: "S/MIME" };

/**
 * Meldung, warum mit OpenPGP bzw. S/MIME (noch) nicht gesendet werden kann, oder `null`. Die
 * Passphrase ist kein Hindernis: Nach ihr wird beim Senden gefragt.
 */
export function versandHindernis(
  pruefung: VersandPruefung,
  verschluesseln: boolean,
  verfahren: Verfahren = "openpgp",
): string | null {
  const smime = verfahren === "smime";
  if (pruefung.eigener_fehlt) {
    return smime
      ? "Für dieses Konto gibt es kein gültiges eigenes S/MIME-Zertifikat. Importiere es unter „Schlüssel und Zertifikate“."
      : "Für dieses Konto gibt es noch keinen eigenen Schlüssel. Erzeuge oder importiere ihn unter „Schlüssel und Zertifikate“.";
  }
  if (verschluesseln && pruefung.fehlende_empfaenger.length > 0) {
    const liste = pruefung.fehlende_empfaenger.join(", ");
    return smime
      ? `Für diese Empfänger fehlt ein gültiges S/MIME-Zertifikat: ${liste}. Ohne Zertifikat wird nicht gesendet – importiere es oder schalte „Verschlüsseln“ aus.`
      : `Für diese Empfänger fehlt ein öffentlicher Schlüssel: ${liste}. Ohne Schlüssel wird nicht gesendet – importiere ihn oder schalte „Verschlüsseln“ aus.`;
  }
  return null;
}

/** Welche Verfahren hat ein Konto (eigener OpenPGP-Schlüssel, gültiges eigenes S/MIME-Zertifikat)? */
export interface EigeneVerfahren {
  openpgp: boolean;
  smime: boolean;
}

/**
 * Voreinstellung: das Verfahren des Entwurfs (z. B. bei Antworten), sofern vorhanden; sonst
 * OpenPGP, wenn ein eigener PGP-Schlüssel da ist, sonst S/MIME.
 */
export function verfahrenWaehlen(vorhanden: EigeneVerfahren, wunsch?: Verfahren): Verfahren {
  if (wunsch && vorhanden[wunsch]) return wunsch;
  if (vorhanden.openpgp) return "openpgp";
  if (vorhanden.smime) return "smime";
  return wunsch ?? "openpgp";
}

/** Lädt, welches Konto welche Verfahren hat (Fehler zählen als „keines“). */
async function verfahrenLaden(): Promise<(konto: string) => EigeneVerfahren> {
  const sicher = async <T>(laden: () => Promise<T>): Promise<T | undefined> => {
    try {
      return await laden();
    } catch {
      return undefined;
    }
  };
  const [pgp, smime] = await Promise.all([sicher(api.pgpUebersicht), sicher(api.smimeUebersicht)]);
  return (konto) => ({
    openpgp: (pgp?.eigene ?? []).some((e) => e.konto === konto),
    smime: (smime?.eigene ?? []).some((e) => e.konto === konto && !e.abgelaufen),
  });
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
  const verschluesseln = el("input", { type: "checkbox", name: "verschluesseln", checked: !!entwurf.verschluesseln });
  const signieren = el("input", { type: "checkbox", name: "signieren", checked: !!entwurf.signieren });
  // Verfahren: Auswahl nur sichtbar, wenn das Konto OpenPGP und S/MIME eingerichtet hat.
  let verfahren: Verfahren = entwurf.verfahren ?? "openpgp";
  const verfahrenWahl = el(
    "select",
    { name: "verfahren", "aria-label": "Verfahren" },
    el("option", { value: "openpgp" }, VERFAHREN_NAME.openpgp),
    el("option", { value: "smime" }, VERFAHREN_NAME.smime),
  );
  const verfahrenFeld = el("label", { class: "umschalter", hidden: true }, "Verfahren:", verfahrenWahl);
  const verschluesselnText = el("span", {});
  const signierenText = el("span", {});
  const pgpHinweis = el("p", { class: "pgp-hinweis", role: "status", "aria-live": "polite" });
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
    el(
      "div",
      { class: "pgp-schalter" },
      verfahrenFeld,
      el("label", { class: "umschalter" }, verschluesseln, verschluesselnText),
      el("label", { class: "umschalter" }, signieren, signierenText),
    ),
    pgpHinweis,
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
    entwurf.verschluesseln = verschluesseln.checked;
    entwurf.signieren = signieren.checked;
    entwurf.verfahren = verfahren;
    return entwurf;
  };

  const verfahrenZeigen = () => {
    verfahrenWahl.value = verfahren;
    verschluesselnText.textContent = `Verschlüsseln (${VERFAHREN_NAME[verfahren]})`;
    signierenText.textContent = `Signieren (${VERFAHREN_NAME[verfahren]})`;
  };
  verfahrenZeigen();
  let vorhandenFuer: ((konto: string) => EigeneVerfahren) | null = null;
  /** Verfahren für das gewählte Konto festlegen; Auswahl nur, wenn beide eingerichtet sind. */
  const verfahrenAnpassen = (wunsch?: Verfahren) => {
    if (!vorhandenFuer) return;
    const vorhanden = vorhandenFuer(von.value);
    verfahren = verfahrenWaehlen(vorhanden, wunsch);
    verfahrenFeld.hidden = !(vorhanden.openpgp && vorhanden.smime);
    verfahrenZeigen();
  };
  const verfahrenBereit = verfahrenLaden().then((fuer) => {
    vorhandenFuer = fuer;
    verfahrenAnpassen(entwurf.verfahren);
  });

  /** Prüft Schlüssel bzw. Zertifikate und zeigt, was fehlt. Gibt die Prüfung zurück (oder `null` ohne Schutz). */
  const pgpPruefen = async (): Promise<VersandPruefung | null> => {
    await verfahrenBereit;
    const daten = lesen();
    if (!daten.verschluesseln && !daten.signieren) {
      pgpHinweis.replaceChildren();
      delete pgpHinweis.dataset.art;
      return null;
    }
    const pruefung = await api.pgpVersandPruefen(daten);
    const hindernis = versandHindernis(pruefung, !!daten.verschluesseln, verfahren);
    if (hindernis) {
      pgpHinweis.textContent = `⚠ ${hindernis}`;
      pgpHinweis.dataset.art = "fehler";
    } else {
      pgpHinweis.textContent = daten.verschluesseln
        ? "🔒 Inhalt und Anhänge werden verschlüsselt. Betreff und Empfänger bleiben sichtbar."
        : "✓ Die Mail wird signiert.";
      pgpHinweis.dataset.art = "info";
    }
    return pruefung;
  };
  const pgpPruefenStill = () => {
    pgpPruefen().catch((fehler) => {
      pgpHinweis.textContent = fehlertext(fehler);
      pgpHinweis.dataset.art = "fehler";
    });
  };
  for (const schalter of [verschluesseln, signieren]) schalter.addEventListener("change", pgpPruefenStill);
  verfahrenWahl.addEventListener("change", () => {
    verfahren = verfahrenWahl.value as Verfahren;
    verfahrenZeigen();
    if (verschluesseln.checked || signieren.checked) pgpPruefenStill();
  });
  von.addEventListener("change", () => verfahrenAnpassen(verfahren));
  for (const empfaenger of [an, cc, bcc, von]) {
    empfaenger.addEventListener("change", () => {
      if (verschluesseln.checked || signieren.checked) pgpPruefenStill();
    });
  }
  if (entwurf.verschluesseln || entwurf.signieren) pgpPruefenStill();
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
    if (daten.verschluesseln || daten.signieren) {
      // Fehlt ein Schlüssel, wird nicht gesendet – nie unbemerkt unverschlüsselt.
      let pruefung: VersandPruefung | null;
      try {
        pruefung = await pgpPruefen();
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
        return;
      }
      if (pruefung) {
        const hindernis = versandHindernis(pruefung, !!daten.verschluesseln, verfahren);
        if (hindernis) {
          statusSetzen(`Nicht gesendet. ${hindernis}`, "fehler");
          return;
        }
        if (pruefung.passphrase_fuer) {
          const smime = verfahren === "smime";
          const passphrase = await passphraseAbfragen({
            titel: "Passphrase zum Signieren",
            text: smime
              ? `Zum Signieren wird die Passphrase deines S/MIME-Zertifikats benötigt.\nZertifikat: ${pruefung.passphrase_fuer}`
              : `Zum Signieren wird die Passphrase deines Schlüssels benötigt.\nSchlüssel: ${pruefung.passphrase_fuer}`,
            ja: "Entsperren und senden",
          });
          if (passphrase === null) return;
          try {
            if (smime) {
              await api.smimeEntsperren(pruefung.passphrase_fuer, passphrase);
            } else {
              await api.pgpEntsperren(pruefung.passphrase_fuer, passphrase);
            }
          } catch (fehler) {
            statusSetzen(fehlertext(fehler), "fehler");
            return;
          }
        }
      }
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

/**
 * Dialog „Schlüssel (OpenPGP)“: eigene Schlüssel pro Konto erzeugen, importieren, exportieren und
 * löschen; öffentliche Schlüssel anderer importieren und löschen.
 *
 * Dateien wählt und speichert ausschliesslich das Backend (Dialoge im Backend); die Oberfläche
 * sieht nie einen privaten Schlüssel. Passphrasen werden nur durchgereicht, nie gespeichert.
 */
import { api, fehlertext, type EigenerSchluessel, type Konto, type SchluesselInfo } from "../api";
import { bestaetigen, modalSchliessen, modalZeigen } from "../dialoge";
import { el } from "../dom";
import { datumLang } from "../postfach/format";

/** Mindestlänge der Passphrase für neue Schlüssel (das Backend prüft ebenfalls). */
export const MIN_PASSPHRASE = 8;

/** Prüft die Passphrase für einen neuen Schlüssel. Gibt eine Meldung oder `null` zurück. */
export function passphrasePruefen(passphrase: string, wiederholung: string): string | null {
  if ([...passphrase].length < MIN_PASSPHRASE) {
    return `Bitte eine Passphrase mit mindestens ${MIN_PASSPHRASE} Zeichen wählen.`;
  }
  if (passphrase !== wiederholung) return "Die beiden Passphrasen stimmen nicht überein.";
  return null;
}

function schluesselAngaben(info: SchluesselInfo): Node[] {
  return [
    el("div", { class: "schluessel-ids" }, info.benutzer_ids.join(", ") || "(ohne User-ID)"),
    el("div", { class: "fingerabdruck" }, el("span", { class: "leise" }, "Fingerabdruck: "), el("code", {}, info.fingerabdruck)),
    el("div", { class: "leise" }, `Erstellt: ${datumLang(info.erstellt)}`),
  ];
}

/** Öffnet den Dialog. */
export async function schluesselOeffnen(konten: Konto[]): Promise<HTMLDialogElement> {
  const status = el("p", { class: "status", role: "status", "aria-live": "polite" });
  const inhalt = el("div", {});
  const schliessen = el("button", { type: "button", class: "haupt" }, "Schliessen");
  const dialog = el(
    "dialog",
    { class: "schluessel-dialog" },
    el("h2", {}, "Schlüssel (OpenPGP)"),
    el(
      "p",
      { class: "leise" },
      "Mit OpenPGP verschlüsselte Mails kann nur lesen, wer den passenden privaten Schlüssel hat. Private Schlüssel liegen verschlüsselt auf diesem Gerät, geschützt mit deiner Passphrase. Schlüsselserver werden nicht abgefragt.",
    ),
    inhalt,
    status,
    el("div", { class: "knoepfe" }, schliessen),
  );
  schliessen.addEventListener("click", () => modalSchliessen(dialog));
  dialog.addEventListener("cancel", (ereignis) => {
    ereignis.preventDefault();
    modalSchliessen(dialog);
  });

  const statusSetzen = (text: string, art: "info" | "erfolg" | "fehler") => {
    status.textContent = text;
    status.dataset.art = art;
  };

  const loeschenFragen = async (info: SchluesselInfo, eigener: boolean) => {
    const ok = await bestaetigen({
      titel: eigener ? "Eigenen Schlüssel löschen?" : "Schlüssel löschen?",
      text: eigener
        ? `Der private Schlüssel ${info.fingerabdruck} wird endgültig von diesem Gerät gelöscht.\nMails, die an diesen Schlüssel verschlüsselt sind, kannst du danach hier nicht mehr lesen – ausser du hast eine Sicherung.`
        : `Der öffentliche Schlüssel von ${info.benutzer_ids[0] ?? info.fingerabdruck} wird gelöscht.`,
      ja: "Endgültig löschen",
      warnung: true,
    });
    if (!ok) return;
    try {
      await api.pgpLoeschen(info.fingerabdruck);
      statusSetzen("Der Schlüssel wurde gelöscht.", "erfolg");
      await laden();
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    }
  };

  const exportieren = async (info: SchluesselInfo) => {
    try {
      const name = await api.pgpExportieren(info.fingerabdruck);
      if (name) statusSetzen(`Öffentlicher Schlüssel gespeichert: ${name}`, "erfolg");
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    }
  };

  const eigenerBereich = (konto: Konto, eigener: EigenerSchluessel | undefined): HTMLElement => {
    const titel = el("h3", {}, `${konto.name} <${konto.email}>`);
    if (eigener) {
      const export_ = el("button", { type: "button", class: "klein" }, "Öffentlichen Schlüssel exportieren …");
      export_.addEventListener("click", () => void exportieren(eigener));
      const loeschen = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
      loeschen.addEventListener("click", () => void loeschenFragen(eigener, true));
      return el(
        "section",
        { class: "einstellung eigener-schluessel" },
        titel,
        ...schluesselAngaben(eigener),
        el(
          "div",
          { class: "leise" },
          eigener.geschuetzt
            ? eigener.entsperrt
              ? "Mit Passphrase geschützt, in dieser Sitzung entsperrt."
              : "Mit Passphrase geschützt."
            : "Achtung: nicht mit einer Passphrase geschützt.",
        ),
        el("div", { class: "konto-knoepfe" }, export_, loeschen),
      );
    }
    const passphrase = el("input", {
      type: "password",
      name: "passphrase",
      autocomplete: "new-password",
      "aria-label": "Passphrase",
      placeholder: "Passphrase (mind. 8 Zeichen)",
    });
    const wiederholung = el("input", {
      type: "password",
      name: "passphrase_wiederholen",
      autocomplete: "new-password",
      "aria-label": "Passphrase wiederholen",
      placeholder: "Passphrase wiederholen",
    });
    const erzeugen = el("button", { type: "button", class: "klein haupt" }, "Schlüssel erzeugen");
    const importieren = el("button", { type: "button", class: "klein" }, "Eigenen Schlüssel importieren …");
    const knoepfe = [erzeugen, importieren];
    const beschaeftigt = (an: boolean) => knoepfe.forEach((k) => (k.disabled = an));
    erzeugen.addEventListener("click", async () => {
      const problem = passphrasePruefen(passphrase.value, wiederholung.value);
      if (problem) {
        statusSetzen(problem, "fehler");
        return;
      }
      beschaeftigt(true);
      statusSetzen("Schlüssel wird erzeugt …", "info");
      try {
        await api.pgpErzeugen(konto.id, passphrase.value);
        statusSetzen("Dein Schlüssel wurde erzeugt. Gib den öffentlichen Schlüssel an deine Kontakte weiter.", "erfolg");
        await laden();
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      } finally {
        passphrase.value = "";
        wiederholung.value = "";
        beschaeftigt(false);
      }
    });
    importieren.addEventListener("click", async () => {
      if (!passphrase.value) {
        statusSetzen("Bitte zuerst die Passphrase des Schlüssels eingeben.", "fehler");
        return;
      }
      beschaeftigt(true);
      try {
        const info = await api.pgpEigenenImportieren(konto.id, passphrase.value);
        if (info) {
          statusSetzen("Dein Schlüssel wurde importiert.", "erfolg");
          await laden();
        }
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      } finally {
        passphrase.value = "";
        wiederholung.value = "";
        beschaeftigt(false);
      }
    });
    return el(
      "section",
      { class: "einstellung eigener-schluessel" },
      titel,
      el("p", { class: "leise" }, "Noch kein eigener Schlüssel. Erzeuge einen neuen (Ed25519/Cv25519) oder importiere einen vorhandenen (ASCII-armored)."),
      el("div", { class: "passphrase-zeile" }, passphrase, wiederholung),
      el("div", { class: "konto-knoepfe" }, ...knoepfe),
    );
  };

  const zeichnen = (eigene: EigenerSchluessel[], fremde: SchluesselInfo[]) => {
    const bereiche: Node[] = konten.map((konto) => eigenerBereich(konto, eigene.find((e) => e.konto === konto.id)));
    // Schlüssel von gelöschten Konten bleiben sichtbar und löschbar.
    for (const verwaist of eigene.filter((e) => !konten.some((k) => k.id === e.konto))) {
      const loeschen = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
      loeschen.addEventListener("click", () => void loeschenFragen(verwaist, true));
      bereiche.push(
        el(
          "section",
          { class: "einstellung eigener-schluessel" },
          el("h3", {}, "Schlüssel eines gelöschten Kontos"),
          ...schluesselAngaben(verwaist),
          el("div", { class: "konto-knoepfe" }, loeschen),
        ),
      );
    }
    if (konten.length === 0) bereiche.push(el("p", { class: "leise" }, "Richte zuerst ein Konto ein."));

    const liste = el("ul", { class: "schluessel-liste" });
    for (const info of fremde) {
      const loeschen = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
      loeschen.addEventListener("click", () => void loeschenFragen(info, false));
      liste.append(el("li", {}, el("div", {}, ...schluesselAngaben(info)), loeschen));
    }
    const importieren = el("button", { type: "button", class: "klein" }, "Öffentlichen Schlüssel importieren …");
    importieren.addEventListener("click", async () => {
      try {
        const neu = await api.pgpOeffentlicheImportieren();
        if (neu.length > 0) {
          statusSetzen(`Importiert: ${neu.map((i) => i.benutzer_ids[0] ?? i.fingerabdruck).join(", ")}`, "erfolg");
          await laden();
        }
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      }
    });
    inhalt.replaceChildren(
      el("h3", { class: "abschnitt" }, "Eigene Schlüssel"),
      ...bereiche,
      el(
        "section",
        { class: "einstellung" },
        el("h3", {}, "Schlüssel anderer"),
        el(
          "p",
          { class: "leise" },
          "Vergleiche den Fingerabdruck mit der Person (z. B. am Telefon), bevor du dich auf einen Schlüssel verlässt.",
        ),
        fremde.length > 0 ? liste : el("p", { class: "leise" }, "Noch keine Schlüssel anderer importiert."),
        importieren,
      ),
    );
  };

  const laden = async () => {
    try {
      const uebersicht = await api.pgpUebersicht();
      zeichnen(uebersicht.eigene, uebersicht.fremde);
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    }
  };

  modalZeigen(dialog);
  await laden();
  return dialog;
}

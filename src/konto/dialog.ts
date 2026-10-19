/** Dialog zum Einrichten und Bearbeiten eines Kontos. */
import { api, fehlertext, type Konto, type Pruefergebnis } from "../api";
import { modalSchliessen, modalZeigen } from "../dialoge";
import { el, finde } from "../dom";
import {
  eingabePruefen,
  formularFuellen,
  formularLesen,
  istStandardPort,
  serverFuellen,
  standardPort,
  type ServerArt,
} from "./formular";

function feld(beschriftung: string, eingabe: HTMLElement, hinweis?: string): HTMLLabelElement {
  return el(
    "label",
    { class: "feld" },
    el("span", { class: "beschriftung" }, beschriftung),
    eingabe,
    hinweis ? el("small", { class: "hinweis" }, hinweis) : null,
  );
}

function serverBereich(art: ServerArt): HTMLFieldSetElement {
  const titel = art === "imap" ? "Posteingang (IMAP)" : "Postausgang (SMTP)";
  const verschluesselung = el(
    "select",
    { name: `${art}_verschluesselung` },
    el("option", { value: "tls" }, "TLS"),
    el("option", { value: "starttls" }, "STARTTLS"),
  );
  const port = el("input", {
    name: `${art}_port`,
    type: "number",
    min: "1",
    max: "65535",
    required: true,
    value: String(standardPort(art, "tls")),
  });
  // Beim Wechsel der Verschlüsselung den passenden Standardport vorschlagen.
  verschluesselung.addEventListener("change", () => {
    const aktuell = Number.parseInt(port.value, 10);
    if (!aktuell || istStandardPort(art, aktuell)) {
      port.value = String(standardPort(art, verschluesselung.value === "starttls" ? "starttls" : "tls"));
    }
  });

  return el(
    "fieldset",
    { class: "server" },
    el("legend", {}, titel),
    feld(
      "Server",
      el("input", {
        name: `${art}_host`,
        type: "text",
        required: true,
        placeholder: art === "imap" ? "imap.example.org" : "smtp.example.org",
        autocomplete: "off",
        spellcheck: "false",
      }),
    ),
    el("div", { class: "zeile" }, feld("Port", port), feld("Verschlüsselung", verschluesselung)),
    el(
      "details",
      { class: "zertifikat" },
      el("summary", {}, "Eigenes Zertifikat hinterlegen (nur für selbstsignierte Server)"),
      el(
        "p",
        { class: "warnung" },
        "Achtung: Ein hinterlegtes Zertifikat ersetzt die normale Prüfung. SicherMail akzeptiert dann nur noch genau dieses Zertifikat. " +
          "Hinterlege es nur, wenn du es selbst vom Serverbetreiber erhalten und geprüft hast.",
      ),
      el("textarea", {
        name: `${art}_zertifikat`,
        rows: "5",
        spellcheck: "false",
        placeholder: "-----BEGIN CERTIFICATE-----\n…\n-----END CERTIFICATE-----",
      }),
    ),
  );
}

/** Text für das Ergebnis des Verbindungstests. */
export function pruefergebnisText(ergebnis: Pruefergebnis): { text: string; erfolg: boolean } {
  const teile = [
    ergebnis.imap ? `IMAP (Empfang): ${ergebnis.imap.meldung}` : "IMAP (Empfang): Verbindung in Ordnung.",
    ergebnis.smtp ? `SMTP (Versand): ${ergebnis.smtp.meldung}` : "SMTP (Versand): Verbindung in Ordnung.",
  ];
  return { text: teile.join("\n"), erfolg: !ergebnis.imap && !ergebnis.smtp };
}

/**
 * Öffnet den Dialog. `konto` ist beim Bearbeiten gesetzt.
 * `gespeichert` wird nach erfolgreichem Speichern aufgerufen.
 */
export function kontoDialogOeffnen(konto: Konto | null, gespeichert: (konto: Konto) => void): HTMLDialogElement {
  const neu = konto === null;
  const status = el("p", { class: "status", role: "status", "aria-live": "polite" });
  const autoKnopf = el("button", { type: "button", class: "klein" }, "Automatisch ausfüllen");
  const testKnopf = el("button", { type: "button", name: "testen" }, "Verbindung testen");
  const speichernKnopf = el("button", { type: "submit", class: "haupt" }, neu ? "Konto anlegen" : "Speichern");
  const abbrechenKnopf = el("button", { type: "button" }, "Abbrechen");

  const form = el(
    "form",
    { method: "dialog", novalidate: true },
    el("h2", {}, neu ? "Konto einrichten" : "Konto bearbeiten"),
    feld("Name", el("input", { name: "name", type: "text", required: true, placeholder: "Anna Muster" })),
    el(
      "div",
      { class: "zeile mit-knopf" },
      feld(
        "E-Mail-Adresse",
        el("input", { name: "email", type: "email", required: true, placeholder: "anna@example.org", autocomplete: "off" }),
      ),
      autoKnopf,
    ),
    feld(
      "Benutzername",
      el("input", { name: "benutzername", type: "text", required: true, autocomplete: "off", spellcheck: "false" }),
    ),
    feld(
      "Passwort",
      el("input", {
        name: "passwort",
        type: "password",
        autocomplete: "off",
        placeholder: neu ? "" : "leer lassen = unverändert",
      }),
      "Das Passwort wird nur im Schlüsselbund deines Betriebssystems gespeichert.",
    ),
    serverBereich("imap"),
    serverBereich("smtp"),
    el(
      "label",
      { class: "umschalter" },
      el("input", { type: "checkbox", name: "gesendet_ablegen", checked: true }),
      "Gesendete Mails im Ordner „Gesendet“ ablegen (ausschalten, wenn der Server das selbst tut)",
    ),
    el(
      "details",
      { class: "erweitert" },
      el("summary", {}, "Erweitert"),
      feld(
        "Server-Kennung für die Absenderprüfung",
        el("input", {
          name: "absender_kennung",
          type: "text",
          autocomplete: "off",
          spellcheck: "false",
          placeholder: "leer = Name des IMAP-Servers",
        }),
        "SicherMail zeigt nur die Absenderprüfung (DKIM, SPF, DMARC) an, die dein Mailserver mit dieser Kennung in die Kopfzeile „Authentication-Results“ schreibt. Meist ist das der Name des IMAP-Servers; sonst steht sie in dieser Kopfzeile ganz vorne.",
      ),
    ),
    status,
    el("div", { class: "knoepfe" }, abbrechenKnopf, testKnopf, speichernKnopf),
  );
  const dialog = el("dialog", { class: "konto-dialog" }, form);

  if (konto) {
    formularFuellen(form, konto);
  } else {
    serverFuellen(form, "smtp", { host: "", port: 587, verschluesselung: "starttls" });
  }

  const statusSetzen = (text: string, art: "info" | "erfolg" | "fehler") => {
    status.textContent = text;
    status.dataset.art = art;
  };
  const beschaeftigt = (aktiv: boolean) => {
    for (const knopf of [autoKnopf, testKnopf, speichernKnopf]) {
      knopf.disabled = aktiv;
    }
  };
  const eingabe = () => formularLesen(form, konto?.id ?? null);

  autoKnopf.addEventListener("click", async () => {
    const email = finde<HTMLInputElement>(form, "[name=email]").value.trim();
    beschaeftigt(true);
    statusSetzen("Suche Einstellungen über HTTPS bei der Domain der Adresse …", "info");
    try {
      const vorschlag = await api.autoconfigSuchen(email);
      if (vorschlag) {
        finde<HTMLInputElement>(form, "[name=benutzername]").value = vorschlag.benutzername;
        serverFuellen(form, "imap", vorschlag.imap);
        serverFuellen(form, "smtp", vorschlag.smtp);
        statusSetzen("Einstellungen gefunden und eingetragen. Bitte prüfen und bei Bedarf anpassen.", "erfolg");
      } else {
        statusSetzen("Keine Einstellungen gefunden. Bitte die Server selbst eintragen.", "info");
      }
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    } finally {
      beschaeftigt(false);
    }
  });

  testKnopf.addEventListener("click", async () => {
    const daten = eingabe();
    const problem = eingabePruefen(daten, neu);
    if (problem) {
      statusSetzen(problem, "fehler");
      return;
    }
    beschaeftigt(true);
    statusSetzen("Verbindung wird getestet …", "info");
    try {
      const { text, erfolg } = pruefergebnisText(await api.kontoTesten(daten));
      statusSetzen(text, erfolg ? "erfolg" : "fehler");
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    } finally {
      beschaeftigt(false);
    }
  });

  form.addEventListener("submit", async (ereignis) => {
    ereignis.preventDefault();
    const daten = eingabe();
    const problem = eingabePruefen(daten, neu);
    if (problem) {
      statusSetzen(problem, "fehler");
      return;
    }
    beschaeftigt(true);
    statusSetzen("Verbindung wird geprüft und Konto gespeichert …", "info");
    try {
      const ergebnis = await api.kontoSpeichern(daten);
      modalSchliessen(dialog);
      gespeichert(ergebnis);
    } catch (fehler) {
      statusSetzen(`Nicht gespeichert. ${fehlertext(fehler)}`, "fehler");
    } finally {
      beschaeftigt(false);
    }
  });

  abbrechenKnopf.addEventListener("click", () => modalSchliessen(dialog));
  dialog.addEventListener("cancel", (ereignis) => {
    ereignis.preventDefault();
    modalSchliessen(dialog);
  });

  modalZeigen(dialog);
  return dialog;
}

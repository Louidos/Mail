import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MailEreignis } from "../src/api";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
let ereignisHoerer: ((e: { payload: MailEreignis }) => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, f: (e: { payload: MailEreignis }) => void) => {
    ereignisHoerer = f;
    return Promise.resolve(() => {});
  },
}));

const { App } = await import("../src/app");

const warten = () => new Promise((r) => setTimeout(r, 0));

const konto = {
  id: "k1",
  name: "Anna",
  email: "anna@example.org",
  benutzername: "anna",
  imap: { host: "imap.example.org", port: 993, verschluesselung: "tls" },
  smtp: { host: "smtp.example.org", port: 465, verschluesselung: "tls" },
};

const ordner = [
  { name: "INBOX", anzeigename: "Posteingang", trenner: "/", sonder: "posteingang", waehlbar: true },
  { name: "Archiv", anzeigename: "Archiv", trenner: "/", sonder: "archiv", waehlbar: true },
];

function mail(uid: number, gelesen = false) {
  return {
    uid,
    betreff: `Betreff ${uid}`,
    absender: "Bea",
    absender_adresse: "bea@example.org",
    datum: 1_700_000_000 + uid,
    gelesen,
    markiert: false,
    groesse: 1234,
  };
}

/** Einfache aufbereitete Mail für Tests, die den Inhalt nicht prüfen. */
const einfacheMail = {
  betreff: "",
  von: [],
  an: [],
  cc: [],
  antwort_an: [],
  datum: null,
  message_id: null,
  referenzen: [],
  text: "Text",
  html: null,
  extern_blockiert: 0,
  extern_geladen: false,
  anhaenge: [],
  absender_pruefung: {
    stufe: "nicht_geprueft",
    kennung: "imap.example.org",
    gefunden: false,
    absender_domain: null,
    dkim: [],
    spf: null,
    dmarc: null,
  },
  pgp: {
    verschluesselung: "keine",
    signatur: "keine",
    signatur_teilweise: false,
    signierer: null,
    signierer_schluessel: null,
    meldung: null,
    passphrase_fuer: null,
    verfahren: "openpgp",
    hinweise: [],
    zertifikat_uebernehmbar: false,
  },
};

/** Antworten des Backends je Befehl. */
function backend(antworten: Record<string, unknown>) {
  invoke.mockImplementation((befehl: string) => {
    const antwort = befehl in antworten ? antworten[befehl] : befehl === "mail_anzeigen" ? einfacheMail : undefined;
    if (antwort instanceof Error) return Promise.reject(antwort);
    return Promise.resolve(typeof antwort === "function" ? (antwort as () => unknown)() : antwort);
  });
}

async function starten(antworten: Record<string, unknown>) {
  backend(antworten);
  const wurzel = document.createElement("div");
  document.body.append(wurzel);
  const app = new App(wurzel);
  await app.starten();
  await warten();
  await warten();
  return { app, wurzel };
}

describe("App", () => {
  beforeEach(() => {
    invoke.mockReset();
    ereignisHoerer = null;
    document.body.replaceChildren();
  });

  it("zeigt ohne Konten eine Begrüssung", async () => {
    const { wurzel } = await starten({ konten_auflisten: [] });
    expect(wurzel.querySelector("h1")?.textContent).toBe("SicherMail");
    expect(wurzel.textContent).toContain("Willkommen bei SicherMail");
  });

  it("zeigt Ordner und öffnet automatisch den Posteingang", async () => {
    const { wurzel } = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: { mails: [mail(2), mail(1, true)], auf_server: 2, offline: null, weitere: false },
    });
    const ordnerKnoepfe = [...wurzel.querySelectorAll(".ordner-knopf")].map((k) => k.textContent);
    expect(ordnerKnoepfe).toEqual(["Posteingang", "Archiv"]);
    expect(invoke).toHaveBeenCalledWith("mails_auflisten", { konto: "k1", ordner: "INBOX", start: 0, anzahl: 50 });
    const zeilen = wurzel.querySelectorAll("button.mail");
    expect(zeilen).toHaveLength(2);
    expect(zeilen[0].classList.contains("ungelesen")).toBe(true);
    expect(zeilen[1].classList.contains("ungelesen")).toBe(false);
  });

  it("markiert eine geöffnete Mail als gelesen", async () => {
    const { wurzel } = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: { mails: [mail(2)], auf_server: 1, offline: null, weitere: false },
      mail_gelesen_setzen: undefined,
    });
    (wurzel.querySelector("button.mail") as HTMLButtonElement).click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mail_gelesen_setzen", { konto: "k1", ordner: "INBOX", uid: 2, gelesen: true });
    expect(wurzel.querySelector("button.mail")?.classList.contains("ungelesen")).toBe(false);
    expect(wurzel.querySelector(".lesen .betreff")?.textContent).toBe("Betreff 2");
  });

  it("lädt die Liste neu, wenn das Backend neue Mails meldet", async () => {
    let aufrufe = 0;
    const { wurzel } = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: () => {
        aufrufe += 1;
        const mails = aufrufe === 1 ? [mail(1)] : [mail(2), mail(1)];
        return { mails, auf_server: mails.length, offline: null, weitere: false };
      },
    });
    expect(wurzel.querySelectorAll("button.mail")).toHaveLength(1);
    ereignisHoerer?.({ payload: { art: "mails_geaendert", konto: "k1", ordner: "INBOX" } });
    await warten();
    expect(wurzel.querySelectorAll("button.mail")).toHaveLength(2);
  });

  it("zeigt Verbindungsabbrüche und den Offline-Zustand an", async () => {
    const { wurzel } = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: { mails: [mail(1)], auf_server: null, offline: "Server nicht erreichbar.", weitere: false },
    });
    expect(wurzel.querySelector(".hinweis-leiste")?.textContent).toContain("gespeicherte Mails");
    ereignisHoerer?.({ payload: { art: "getrennt", konto: "k1", meldung: "Verbindung unterbrochen.", sekunden: 4 } });
    expect(wurzel.querySelector(".statusleiste")?.textContent).toContain("Neuer Versuch in 4 s");
    ereignisHoerer?.({ payload: { art: "verbunden", konto: "k1", idle: true } });
    expect(wurzel.querySelector(".statusleiste")?.textContent).toBe("");
  });

  it("zeigt lokal gespeicherte Mails sofort, auch wenn der Server nicht antwortet", async () => {
    backend({
      konten_auflisten: [konto],
      ordner_auflisten: () => new Promise(() => {}),
      mails_lokal: { mails: [mail(3, true)], auf_server: null, offline: null, weitere: false },
      mails_auflisten: () => new Promise(() => {}),
    });
    const wurzel = document.createElement("div");
    document.body.append(wurzel);
    // Nicht abwarten: Der Server antwortet in diesem Test nie.
    void new App(wurzel).starten();
    for (let i = 0; i < 5; i++) await warten();
    expect(wurzel.querySelectorAll("button.mail")).toHaveLength(1);
    expect(wurzel.querySelector(".mail-betreff")?.textContent).toBe("Betreff 3");
    // Der Posteingang ist ohne Ordnerliste vom Server wählbar.
    expect(wurzel.querySelector(".ordner-knopf")?.textContent).toBe("Posteingang");
    expect(wurzel.textContent).toContain("Mails werden geladen");
  });

  it("lädt ältere Mails nach", async () => {
    const { wurzel } = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: { mails: [mail(9)], auf_server: 10, offline: null, weitere: true },
    });
    const mehr = [...wurzel.querySelectorAll("button")].find((k) => k.textContent === "Ältere Mails laden");
    mehr?.click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mails_auflisten", { konto: "k1", ordner: "INBOX", start: 1, anzahl: 50 });
  });

  it("setzt Texte nie als HTML", async () => {
    const { wurzel } = await starten({
      konten_auflisten: [{ ...konto, name: "<img src=x>" }],
      ordner_auflisten: ordner,
      mails_auflisten: {
        mails: [{ ...mail(1), betreff: "<img src=y onerror=alert(1)>" }],
        auf_server: 1,
        offline: null,
        weitere: false,
      },
    });
    expect(wurzel.querySelector("img")).toBeNull();
    expect(wurzel.textContent).toContain("<img src=x>");
    expect(wurzel.textContent).toContain("<img src=y onerror=alert(1)>");
  });
});

describe("Löschen", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  async function oeffnenUndLoeschen(ordnerListe: unknown[]) {
    const { wurzel } = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordnerListe,
      mails_auflisten: { mails: [mail(1, true)], auf_server: 1, offline: null, weitere: false },
      mail_loeschen: undefined,
    });
    (wurzel.querySelector("button.mail") as HTMLButtonElement).click();
    await warten();
    const loeschen = [...wurzel.querySelectorAll(".lesen button")].find((k) => k.textContent === "Löschen");
    (loeschen as HTMLButtonElement).click();
    await warten();
    return wurzel;
  }

  it("verschiebt ohne Rückfrage, wenn es einen Papierkorb gibt", async () => {
    const wurzel = await oeffnenUndLoeschen([
      ...ordner,
      { name: "Trash", anzeigename: "Trash", trenner: "/", sonder: "papierkorb", waehlbar: true },
    ]);
    expect(invoke).toHaveBeenCalledWith("mail_loeschen", { konto: "k1", ordner: "INBOX", uid: 1 });
    expect(wurzel.querySelectorAll("button.mail")).toHaveLength(0);
  });

  it("fragt nach, wenn ohne Papierkorb endgültig gelöscht würde", async () => {
    await oeffnenUndLoeschen(ordner);
    expect(invoke).not.toHaveBeenCalledWith("mail_loeschen", expect.anything());
    const dialog = document.querySelector("dialog.bestaetigen");
    expect(dialog?.textContent).toContain("endgültig");
    const ja = [...dialog!.querySelectorAll("button")].find((k) => k.textContent === "Endgültig löschen");
    (ja as HTMLButtonElement).click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mail_loeschen", { konto: "k1", ordner: "INBOX", uid: 1 });
  });
});

describe("Leseansicht", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  const aufbereitet = {
    betreff: "Angebot",
    von: [{ name: "Bea", adresse: "bea@example.org" }],
    an: [{ name: null, adresse: "anna@example.org" }],
    cc: [],
    antwort_an: [],
    datum: 1_700_000_000,
    message_id: "m1@example.org",
    referenzen: [],
    text: "Hallo Anna",
    html: "<p>Hallo <b>Anna</b></p>",
    extern_blockiert: 2,
    extern_geladen: false,
    anhaenge: [{ index: 0, name: "Rechnung.pdf", typ: "application/pdf", groesse: 2048 }],
    absender_pruefung: {
      stufe: "bestaetigt",
      kennung: "mail.example.org",
      gefunden: true,
      absender_domain: "example.org",
      dkim: [{ ergebnis: "pass", domain: "example.org", passend: true }],
      spf: { ergebnis: "pass", domain: "example.org", passend: true },
      dmarc: { ergebnis: "pass", domain: "example.org", passend: true },
    },
    pgp: {
      verschluesselung: "keine",
      signatur: "keine",
      signatur_teilweise: false,
      signierer: null,
      signierer_schluessel: null,
      meldung: null,
      passphrase_fuer: null,
      verfahren: "openpgp",
      hinweise: [],
      zertifikat_uebernehmbar: false,
    },
  };

  const gefaelscht = {
    ...aufbereitet,
    absender_pruefung: {
      stufe: "warnung",
      kennung: "mail.example.org",
      gefunden: true,
      absender_domain: "example.org",
      dkim: [{ ergebnis: "fail", domain: "example.org", passend: true }],
      spf: { ergebnis: "fail", domain: "example.org", passend: true },
      dmarc: { ergebnis: "fail", domain: "example.org", passend: true },
    },
  };

  async function oeffnen(antworten: Record<string, unknown> = {}) {
    const { wurzel, app } = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: { mails: [mail(5, true)], auf_server: 1, offline: null, weitere: false },
      mail_anzeigen: aufbereitet,
      ...antworten,
    });
    (wurzel.querySelector("button.mail") as HTMLButtonElement).click();
    await warten();
    await warten();
    return { wurzel, app };
  }

  it("zeigt HTML nur im abgeschotteten iframe und blockiert externe Inhalte", async () => {
    const { wurzel } = await oeffnen();
    expect(invoke).toHaveBeenCalledWith("mail_anzeigen", { konto: "k1", ordner: "INBOX", uid: 5, externLaden: false });
    const rahmen = wurzel.querySelector("iframe.mail-rahmen") as HTMLIFrameElement;
    expect(rahmen.getAttribute("sandbox")).toBe("allow-same-origin");
    expect(rahmen.srcdoc).toContain("<b>Anna</b>");
    expect(wurzel.querySelector(".lesebereich")?.innerHTML).not.toContain("<b>Anna</b></p></div>");
    expect(wurzel.querySelector(".extern-leiste")?.textContent).toContain("2 externe Inhalte wurden blockiert");
  });

  it("lädt externe Inhalte erst nach Klick", async () => {
    const { wurzel } = await oeffnen();
    const laden = [...wurzel.querySelectorAll(".extern-leiste button")].find(
      (k) => k.textContent === "Externe Inhalte laden",
    ) as HTMLButtonElement;
    laden.click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mail_anzeigen", { konto: "k1", ordner: "INBOX", uid: 5, externLaden: true });
  });

  it("kann externe Inhalte für einen Absender immer erlauben", async () => {
    const { wurzel } = await oeffnen({ absender_extern_erlauben: undefined });
    const immer = [...wurzel.querySelectorAll(".extern-leiste button")].find((k) =>
      k.textContent?.includes("immer erlauben"),
    ) as HTMLButtonElement;
    immer.click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("absender_extern_erlauben", { adresse: "bea@example.org" });
  });

  it("zeigt mit „Nur Text“ den Text statt des HTML", async () => {
    const { wurzel } = await oeffnen();
    const umschalter = wurzel.querySelector(".umschalter input") as HTMLInputElement;
    umschalter.checked = true;
    umschalter.dispatchEvent(new Event("change"));
    expect(wurzel.querySelector("iframe")).toBeNull();
    expect(wurzel.querySelector(".mailtext")?.textContent).toBe("Hallo Anna");
    expect(wurzel.querySelector(".extern-leiste")).toBeNull();
  });

  it("listet Anhänge und speichert nur über „Speichern unter“", async () => {
    const { wurzel } = await oeffnen({ anhang_speichern: "Rechnung.pdf" });
    expect(wurzel.querySelector(".anhang-name")?.textContent).toBe("Rechnung.pdf");
    const knopf = [...wurzel.querySelectorAll(".anhaenge button")].find(
      (k) => k.textContent === "Speichern unter …",
    ) as HTMLButtonElement;
    knopf.click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("anhang_speichern", { konto: "k1", ordner: "INBOX", uid: 5, index: 0 });
    expect(wurzel.textContent).toContain("Gespeichert: Rechnung.pdf");
  });

  it("warnt vor Links mit abweichender Zieladresse und öffnet erst nach Bestätigung", async () => {
    const { app } = await oeffnen({ link_oeffnen: undefined });
    const klick = app.linkKlick({ href: "https://boese.example/login", text: "https://meine-bank.ch" });
    await warten();
    const dialog = document.querySelector("dialog.bestaetigen")!;
    expect(dialog.textContent).toContain("verdächtiger Link");
    expect(dialog.textContent).toContain("https://boese.example/login");
    expect(invoke).not.toHaveBeenCalledWith("link_oeffnen", expect.anything());
    ([...dialog.querySelectorAll("button")].find((k) => k.textContent === "Trotzdem öffnen") as HTMLButtonElement).click();
    await klick;
    expect(invoke).toHaveBeenCalledWith("link_oeffnen", { url: "https://boese.example/login" });
  });

  it("öffnet unverdächtige Links direkt im Standardbrowser", async () => {
    const { app } = await oeffnen({ link_oeffnen: undefined });
    await app.linkKlick({ href: "https://example.org/", text: "Zur Webseite" });
    expect(document.querySelector("dialog.bestaetigen")).toBeNull();
    expect(invoke).toHaveBeenCalledWith("link_oeffnen", { url: "https://example.org/" });
  });

  it("zeigt beim Absender, dass der eigene Server ihn bestätigt hat, mit Einzelheiten", async () => {
    const { wurzel } = await oeffnen();
    const feld = wurzel.querySelector("details.absender-pruefung") as HTMLDetailsElement;
    expect(feld.dataset.stufe).toBe("bestaetigt");
    expect(feld.querySelector("summary")?.textContent).toBe("✓Absender bestätigt");
    expect(feld.textContent).toContain("DKIM: pass für example.org");
    expect(feld.textContent).toContain("SPF: pass für example.org");
    expect(feld.textContent).toContain("DMARC: pass für example.org");
    expect(feld.textContent).toContain("Geprüft von: mail.example.org");
    expect(wurzel.querySelector(".absender-warnung")).toBeNull();
  });

  it("zeigt „nicht geprüft“, wenn der eigene Server kein Ergebnis liefert", async () => {
    const { wurzel } = await oeffnen({
      mail_anzeigen: {
        ...aufbereitet,
        absender_pruefung: { ...einfacheMail.absender_pruefung, kennung: "mail.example.org" },
      },
    });
    const feld = wurzel.querySelector("details.absender-pruefung") as HTMLDetailsElement;
    expect(feld.dataset.stufe).toBe("nicht_geprueft");
    expect(feld.querySelector("summary")?.textContent).toContain("Absender nicht geprüft");
    expect(feld.textContent).toContain("Kein Prüfergebnis deines Servers (Kennung „mail.example.org“) gefunden.");
  });

  it("warnt deutlich vor gefälschten Absendern und bietet keine dauerhafte Freigabe an", async () => {
    const { wurzel } = await oeffnen({ mail_anzeigen: gefaelscht });
    const warnung = wurzel.querySelector(".absender-warnung") as HTMLElement;
    expect(warnung.getAttribute("role")).toBe("alert");
    expect(warnung.textContent).toContain("⚠");
    expect(warnung.textContent).toContain(
      "Der Absender ist wahrscheinlich gefälscht. Antworte nicht, öffne keine Links und Anhänge, wenn du unsicher bist.",
    );
    const feld = wurzel.querySelector("details.absender-pruefung") as HTMLDetailsElement;
    expect(feld.querySelector("summary")?.textContent).toContain("Absender wahrscheinlich gefälscht");
    const knoepfe = [...wurzel.querySelectorAll(".extern-leiste button")].map((k) => k.textContent);
    expect(knoepfe).toEqual(["Externe Inhalte laden"]);
    expect(wurzel.textContent).not.toContain("immer erlauben");
  });

  const pgpLeer = einfacheMail.pgp;

  it("zeigt entschlüsselte und gültig signierte Mails deutlich an", async () => {
    const { wurzel } = await oeffnen({
      mail_anzeigen: {
        ...aufbereitet,
        pgp: { ...pgpLeer, verschluesselung: "entschluesselt", signatur: "gueltig", signierer: "Bea <bea@example.org>" },
      },
    });
    const leiste = wurzel.querySelector(".pgp-leiste") as HTMLElement;
    expect(leiste.textContent).toContain("Verschlüsselt");
    expect(leiste.textContent).toContain("Signiert und gültig (Schlüssel von Bea <bea@example.org>)");
  });

  it("kennzeichnet ungültige, fremde, unbekannte und teilweise Signaturen", async () => {
    const faelle: [Record<string, unknown>, string][] = [
      [{ signatur: "ungueltig" }, "Signatur ungültig"],
      [{ signatur: "unbekannt", signierer_schluessel: "ABCD 1234" }, "Schlüssel unbekannt (ABCD 1234)"],
      [{ signatur: "andere_adresse", signierer: "Carl <carl@example.org>" }, "gehört nicht zur Absenderadresse"],
      [{ signatur: "gueltig", signatur_teilweise: true }, "Nur ein Teil dieser Mail ist signiert"],
      [{ verschluesselung: "teilweise", meldung: "Nur ein Teil" }, "Nur teilweise verschlüsselt"],
    ];
    for (const [pgp, text] of faelle) {
      invoke.mockReset();
      document.body.replaceChildren();
      const { wurzel } = await oeffnen({ mail_anzeigen: { ...aufbereitet, pgp: { ...pgpLeer, ...pgp } } });
      expect(wurzel.querySelector(".pgp-leiste")?.textContent).toContain(text);
    }
  });

  it("fragt bei verschlüsselten Mails nach der Passphrase und lädt danach neu", async () => {
    const { wurzel } = await oeffnen({
      mail_anzeigen: {
        ...aufbereitet,
        pgp: {
          ...pgpLeer,
          verschluesselung: "nicht_entschluesselt",
          meldung: "Zum Entschlüsseln wird die Passphrase deines Schlüssels benötigt.",
          passphrase_fuer: "AAAA BBBB",
        },
      },
      pgp_entsperren: undefined,
    });
    const leiste = wurzel.querySelector(".pgp-leiste") as HTMLElement;
    expect(leiste.textContent).toContain("Kann nicht entschlüsselt werden");
    const feld = leiste.querySelector("[name=pgp_passphrase]") as HTMLInputElement;
    expect(feld.type).toBe("password");
    feld.value = "geheime-passphrase";
    (leiste.querySelector("form") as HTMLFormElement).dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_entsperren", { fingerabdruck: "AAAA BBBB", passphrase: "geheime-passphrase" });
    expect(feld.value).toBe("");
    const aufrufe = invoke.mock.calls.filter(([befehl]) => befehl === "mail_anzeigen");
    expect(aufrufe.length).toBe(2);
  });

  it("bietet bei Schlüssel-Anhängen den Import an", async () => {
    const { wurzel } = await oeffnen({
      mail_anzeigen: {
        ...aufbereitet,
        anhaenge: [{ index: 0, name: "bea.asc", typ: "application/pgp-keys", groesse: 900 }],
      },
      pgp_aus_anhang_importieren: [{ fingerabdruck: "AAAA", benutzer_ids: ["Bea <bea@example.org>"], adressen: [], erstellt: 0, kann_verschluesseln: true }],
    });
    const knopf = [...wurzel.querySelectorAll(".anhaenge button")].find(
      (k) => k.textContent === "Schlüssel importieren",
    ) as HTMLButtonElement;
    knopf.click();
    await warten();
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_aus_anhang_importieren", { konto: "k1", ordner: "INBOX", uid: 5, index: 0 });
    expect(wurzel.textContent).toContain("Importiert: Bea <bea@example.org>");
  });

  it("zeigt S/MIME-Ergebnisse mit Verfahren, Status und Hinweisen", async () => {
    const smime = { ...pgpLeer, verfahren: "smime" };
    const faelle: [Record<string, unknown>, string][] = [
      [
        { verschluesselung: "entschluesselt", signatur: "gueltig", signierer: "Bea Test <bea@example.org>" },
        "Mit S/MIME signiert und gültig (Zertifikat von Bea Test <bea@example.org>)",
      ],
      [{ verschluesselung: "entschluesselt" }, "Verschlüsselt (S/MIME)"],
      [{ signatur: "unbekannt", signierer_schluessel: "ABCD 1234" }, "von einer unbekannten Zertifizierungsstelle"],
      [{ signatur: "abgelaufen" }, "Zertifikat war beim Signieren abgelaufen"],
      [{ signatur: "andere_adresse" }, "das Zertifikat gehört nicht zur Absenderadresse"],
      [{ signatur: "ungueltig" }, "Signatur ungültig"],
      [{ signatur: "gueltig", signatur_teilweise: true }, "Nur ein Teil dieser Mail ist signiert"],
      [{ verschluesselung: "teilweise" }, "Nur teilweise verschlüsselt"],
      [{ signatur: "gueltig", hinweise: ["Sperrstatus nicht online geprüft"] }, "Sperrstatus nicht online geprüft"],
    ];
    for (const [pgp, text] of faelle) {
      invoke.mockReset();
      document.body.replaceChildren();
      const { wurzel } = await oeffnen({ mail_anzeigen: { ...aufbereitet, pgp: { ...smime, ...pgp } } });
      expect(wurzel.querySelector(".pgp-leiste")?.textContent).toContain(text);
    }
  });

  it("markiert den CBC-Hinweis als Warnung", async () => {
    const { wurzel } = await oeffnen({
      mail_anzeigen: {
        ...aufbereitet,
        pgp: {
          ...pgpLeer,
          verfahren: "smime",
          verschluesselung: "entschluesselt",
          hinweise: ["Verschlüsselt, aber nicht gegen Veränderung geschützt (AES-CBC ohne gültige Signatur)."],
        },
      },
    });
    const zeilen = [...wurzel.querySelectorAll(".pgp-zeile")] as HTMLElement[];
    const hinweis = zeilen.find((z) => z.textContent?.includes("nicht gegen Veränderung"));
    expect(hinweis?.dataset.art).toBe("warnung");
  });

  it("übernimmt das S/MIME-Zertifikat aus einer gültig signierten Mail", async () => {
    const { wurzel } = await oeffnen({
      mail_anzeigen: {
        ...aufbereitet,
        pgp: { ...pgpLeer, verfahren: "smime", signatur: "gueltig", zertifikat_uebernehmbar: true },
      },
      smime_aus_mail_uebernehmen: {
        fingerabdruck: "AAAA",
        name: "Bea Test",
        adressen: ["bea@example.org"],
        aussteller: "Test-Stelle",
        gueltig_ab: 0,
        gueltig_bis: 2_000_000_000,
        abgelaufen: false,
      },
    });
    const knopf = [...wurzel.querySelectorAll(".pgp-leiste button")].find(
      (k) => k.textContent === "Zertifikat übernehmen",
    ) as HTMLButtonElement;
    knopf.click();
    await warten();
    await warten();
    expect(invoke).toHaveBeenCalledWith("smime_aus_mail_uebernehmen", { konto: "k1", ordner: "INBOX", uid: 5 });
    expect(wurzel.textContent).toContain("Zertifikat übernommen: Bea Test (bea@example.org)");
  });

  it("entsperrt bei S/MIME das Zertifikat statt eines OpenPGP-Schlüssels", async () => {
    const { wurzel } = await oeffnen({
      mail_anzeigen: {
        ...aufbereitet,
        pgp: { ...pgpLeer, verfahren: "smime", verschluesselung: "nicht_entschluesselt", passphrase_fuer: "CCCC DDDD" },
      },
    });
    const leiste = wurzel.querySelector(".pgp-leiste") as HTMLElement;
    expect(leiste.textContent).toContain("Zertifikat CCCC DDDD");
    expect(leiste.textContent).not.toContain("Zertifikat übernehmen");
    (leiste.querySelector("[name=pgp_passphrase]") as HTMLInputElement).value = "test-passwort";
    (leiste.querySelector("form") as HTMLFormElement).dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(invoke).toHaveBeenCalledWith("smime_entsperren", { fingerabdruck: "CCCC DDDD", passphrase: "test-passwort" });
    expect(invoke).not.toHaveBeenCalledWith("pgp_entsperren", expect.anything());
  });

  it("zeigt bei normalen Mails keine OpenPGP-Leiste", async () => {
    const { wurzel } = await oeffnen();
    expect(wurzel.querySelector(".pgp-leiste")).toBeNull();
    expect(wurzel.textContent).not.toContain("Schlüssel importieren");
  });
});

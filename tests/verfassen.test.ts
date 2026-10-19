import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AufbereiteteMail, Konto } from "../src/api";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { antwortEntwurf, betreffMitVorsilbe, leererEntwurf, weiterleitenEntwurf, zitieren, adresseFormatieren } =
  await import("../src/verfassen/zitat");
const { verfassenOeffnen } = await import("../src/verfassen/dialog");

const warten = () => new Promise((r) => setTimeout(r, 0));

const konto: Konto = {
  id: "k1",
  name: "Anna",
  email: "anna@example.org",
  benutzername: "anna",
  imap: { host: "imap.example.org", port: 993, verschluesselung: "tls" },
  smtp: { host: "smtp.example.org", port: 465, verschluesselung: "tls" },
  gesendet_ablegen: true,
  absender_kennung: "",
};

const mail: AufbereiteteMail = {
  betreff: "Treffen am Freitag",
  von: [{ name: "Bea", adresse: "bea@example.org" }],
  an: [
    { name: "Anna", adresse: "Anna@example.org" },
    { name: "Muster, Carl", adresse: "carl@example.org" },
  ],
  cc: [{ name: null, adresse: "dora@example.org" }],
  antwort_an: [],
  datum: 1_700_000_000,
  message_id: "treffen-2@example.org",
  referenzen: ["start-1@example.org"],
  text: "Hallo zusammen\n> alte Zeile\nBis bald",
  html: null,
  extern_blockiert: 0,
  extern_geladen: false,
  anhaenge: [],
  absender_pruefung: {
    stufe: "nicht_geprueft",
    kennung: "imap.example.org",
    gefunden: false,
    absender_domain: "example.org",
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
  },
};

describe("Antworten und Weiterleiten", () => {
  it("setzt Vorsilben nur einmal", () => {
    expect(betreffMitVorsilbe("Hallo", "Re")).toBe("Re: Hallo");
    expect(betreffMitVorsilbe("Re: Hallo", "Re")).toBe("Re: Hallo");
    expect(betreffMitVorsilbe("AW: Hallo", "Re")).toBe("AW: Hallo");
    expect(betreffMitVorsilbe("Hallo", "Fwd")).toBe("Fwd: Hallo");
    expect(betreffMitVorsilbe("WG: Hallo", "Fwd")).toBe("WG: Hallo");
  });

  it("zitiert mit Einleitung und „> “", () => {
    const zitat = zitieren(mail);
    expect(zitat).toMatch(/^Am .+ schrieb Bea <bea@example.org>:\n/);
    expect(zitat).toContain("> Hallo zusammen\n>> alte Zeile\n> Bis bald");
  });

  it("antwortet dem Absender mit In-Reply-To und References", () => {
    const e = antwortEntwurf(mail, konto, false);
    expect(e.an).toBe("Bea <bea@example.org>");
    expect(e.cc).toBe("");
    expect(e.betreff).toBe("Re: Treffen am Freitag");
    expect(e.antwort_auf).toBe("treffen-2@example.org");
    expect(e.referenzen).toEqual(["start-1@example.org"]);
    expect(e.text.startsWith("\n\nAm ")).toBe(true);
  });

  it("antwortet allen ausser sich selbst, Namen mit Komma in Anführungszeichen", () => {
    const e = antwortEntwurf(mail, konto, true);
    expect(e.an).toBe('Bea <bea@example.org>, "Muster, Carl" <carl@example.org>');
    expect(e.cc).toBe("dora@example.org");
  });

  it("verwendet Reply-To, wenn vorhanden", () => {
    const e = antwortEntwurf({ ...mail, antwort_an: [{ name: null, adresse: "liste@example.org" }] }, konto, false);
    expect(e.an).toBe("liste@example.org");
  });

  it("leitet mit Kopf der Originalmail weiter", () => {
    const e = weiterleitenEntwurf(mail, konto);
    expect(e.an).toBe("");
    expect(e.betreff).toBe("Fwd: Treffen am Freitag");
    expect(e.text).toContain("---------- Weitergeleitete Nachricht ----------");
    expect(e.text).toContain("Von: Bea <bea@example.org>");
    expect(e.text).toContain("Hallo zusammen");
    expect(e.antwort_auf).toBeUndefined();
  });

  it("wählt bei Antworten auf verschlüsselte Mails „Verschlüsseln“ vor", () => {
    expect(antwortEntwurf(mail, konto, false).verschluesseln).toBe(false);
    const verschluesselt = { ...mail, pgp: { ...mail.pgp, verschluesselung: "entschluesselt" as const } };
    const antwort = antwortEntwurf(verschluesselt, konto, false);
    expect(antwort.verschluesseln).toBe(true);
    expect(antwort.signieren).toBe(true);
    expect(weiterleitenEntwurf(verschluesselt, konto).verschluesseln).toBe(true);
  });

  it("formatiert Adressen", () => {
    expect(adresseFormatieren({ name: null, adresse: "a@example.org" })).toBe("a@example.org");
    expect(adresseFormatieren({ name: "A. B.", adresse: "a@example.org" })).toBe('"A. B." <a@example.org>');
  });
});

describe("Verfassen-Fenster", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  function knopf(text: string): HTMLButtonElement {
    return [...document.querySelectorAll("button")].find((k) => k.textContent === text) as HTMLButtonElement;
  }

  it("sichert den Entwurf vor dem Senden und schliesst nach Erfolg", async () => {
    invoke.mockImplementation((befehl: string, args: { entwurf?: object }) => {
      if (befehl === "entwurf_speichern") return Promise.resolve({ ...args.entwurf, id: "e1" });
      if (befehl === "mail_senden") return Promise.resolve({ abgelegt: true, hinweis: null });
      return Promise.resolve(undefined);
    });
    const gesendet = vi.fn();
    const dialog = verfassenOeffnen({ konten: [konto], entwurf: { ...leererEntwurf("k1"), an: "bea@example.org", betreff: "Hallo" }, gesendet });
    (dialog.querySelector("[name=text]") as HTMLTextAreaElement).value = "Text";
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    const befehle = invoke.mock.calls.map((c) => c[0]);
    expect(befehle).toEqual(["entwurf_speichern", "mail_senden"]);
    expect(invoke.mock.calls[1][1].entwurf).toMatchObject({ id: "e1", an: "bea@example.org", text: "Text" });
    expect(gesendet).toHaveBeenCalledWith({ abgelegt: true, hinweis: null });
    expect(document.body.contains(dialog)).toBe(false);
  });

  it("behält Text und Fenster, wenn das Senden scheitert", async () => {
    invoke.mockImplementation((befehl: string, args: { entwurf?: object }) => {
      if (befehl === "entwurf_speichern") return Promise.resolve({ ...args.entwurf, id: "e1" });
      if (befehl === "mail_senden")
        return Promise.reject({ art: "nicht_erreichbar", meldung: "Server nicht erreichbar. Die Mail wurde nicht gesendet und als Entwurf gespeichert." });
      return Promise.resolve(undefined);
    });
    const dialog = verfassenOeffnen({ konten: [konto], entwurf: { ...leererEntwurf("k1"), an: "bea@example.org", betreff: "Hallo" } });
    const text = dialog.querySelector("[name=text]") as HTMLTextAreaElement;
    text.value = "Wichtiger Text";
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    expect(document.body.contains(dialog)).toBe(true);
    expect(text.value).toBe("Wichtiger Text");
    expect(dialog.querySelector(".status")?.textContent).toContain("als Entwurf gespeichert");
  });

  it("sendet ohne Empfänger nichts", async () => {
    const dialog = verfassenOeffnen({ konten: [konto], entwurf: leererEntwurf("k1") });
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(invoke).not.toHaveBeenCalled();
    expect(dialog.querySelector(".status")?.textContent).toContain("Empfänger");
  });

  it("hängt Dateien über das Backend an und kann sie entfernen", async () => {
    invoke.mockResolvedValue([{ token: "t1", name: "Bericht.pdf", typ: "application/pdf", groesse: 2048 }]);
    const dialog = verfassenOeffnen({ konten: [konto], entwurf: leererEntwurf("k1") });
    knopf("Datei anhängen …").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("anhang_waehlen");
    expect(dialog.querySelector(".anhang-name")?.textContent).toBe("Bericht.pdf");
    knopf("Entfernen").click();
    expect(dialog.querySelector(".anhang-name")).toBeNull();
  });

  function pgpBackend(pruefung: object, gesendet: string[] = []) {
    invoke.mockImplementation((befehl: string, args: { entwurf?: object }) => {
      gesendet.push(befehl);
      if (befehl === "pgp_versand_pruefen") return Promise.resolve(pruefung);
      if (befehl === "entwurf_speichern") return Promise.resolve({ ...args.entwurf, id: "e1" });
      if (befehl === "mail_senden") return Promise.resolve({ abgelegt: true, hinweis: null });
      return Promise.resolve(undefined);
    });
  }

  const ohneHindernis = { fehlende_empfaenger: [], eigener_fehlt: false, passphrase_fuer: null };

  it("sendet verschlüsselt nicht, wenn ein Empfängerschlüssel fehlt, und nennt ihn", async () => {
    pgpBackend({ ...ohneHindernis, fehlende_empfaenger: ["carl@example.org"] });
    const dialog = verfassenOeffnen({
      konten: [konto],
      entwurf: { ...leererEntwurf("k1"), an: "bea@example.org, carl@example.org", betreff: "Geheim", verschluesseln: true },
    });
    await warten();
    expect((dialog.querySelector("[name=verschluesseln]") as HTMLInputElement).checked).toBe(true);
    expect(dialog.querySelector(".pgp-hinweis")?.textContent).toContain("carl@example.org");
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("mail_senden", expect.anything());
    expect(dialog.querySelector(".status")?.textContent).toMatch(/Nicht gesendet.*carl@example.org/);
    expect(document.body.contains(dialog)).toBe(true);
  });

  it("verlangt einen eigenen Schlüssel", async () => {
    pgpBackend({ ...ohneHindernis, eigener_fehlt: true });
    const dialog = verfassenOeffnen({
      konten: [konto],
      entwurf: { ...leererEntwurf("k1"), an: "bea@example.org", betreff: "x", signieren: true },
    });
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("mail_senden", expect.anything());
    expect(dialog.querySelector(".status")?.textContent).toContain("keinen eigenen Schlüssel");
  });

  it("fragt zum Signieren nach der Passphrase und sendet erst danach", async () => {
    const befehle: string[] = [];
    pgpBackend({ ...ohneHindernis, passphrase_fuer: "AAAA 1111" }, befehle);
    const gesendet = vi.fn();
    const dialog = verfassenOeffnen({
      konten: [konto],
      entwurf: { ...leererEntwurf("k1"), an: "bea@example.org", betreff: "x", verschluesseln: true, signieren: true },
      gesendet,
    });
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    const frage = document.querySelector("dialog.passphrase") as HTMLDialogElement;
    expect(frage.textContent).toContain("AAAA 1111");
    (frage.querySelector("input[type=password]") as HTMLInputElement).value = "meine-passphrase";
    frage.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_entsperren", { fingerabdruck: "AAAA 1111", passphrase: "meine-passphrase" });
    expect(befehle.indexOf("pgp_entsperren")).toBeLessThan(befehle.indexOf("mail_senden"));
    const senden = invoke.mock.calls.find((c) => c[0] === "mail_senden")!;
    expect(senden[1].entwurf).toMatchObject({ verschluesseln: true, signieren: true });
    expect(gesendet).toHaveBeenCalled();
  });

  it("sendet nicht, wenn die Passphrase-Abfrage abgebrochen wird", async () => {
    pgpBackend({ ...ohneHindernis, passphrase_fuer: "AAAA 1111" });
    const dialog = verfassenOeffnen({
      konten: [konto],
      entwurf: { ...leererEntwurf("k1"), an: "bea@example.org", betreff: "x", signieren: true },
    });
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    knopf("Abbrechen").click();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("mail_senden", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("pgp_entsperren", expect.anything());
  });

  it("schliesst nicht mit Escape, damit kein Text verloren geht", () => {
    const dialog = verfassenOeffnen({ konten: [konto], entwurf: leererEntwurf("k1") });
    const ereignis = new Event("cancel", { cancelable: true });
    dialog.dispatchEvent(ereignis);
    expect(ereignis.defaultPrevented).toBe(true);
    expect(document.body.contains(dialog)).toBe(true);
  });
});

describe("Entwürfe-Liste", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  it("zeigt einen Fehler an, wenn das Löschen eines Entwurfs scheitert", async () => {
    const { entwuerfeOeffnen } = await import("../src/verfassen/entwuerfe");
    const entwurf = { ...leererEntwurf("k1"), id: "e1", betreff: "Alt" };
    invoke.mockImplementation((befehl: string) => {
      if (befehl === "entwuerfe_auflisten") return Promise.resolve([entwurf]);
      if (befehl === "entwurf_loeschen") return Promise.reject({ art: "speicher", meldung: "Speicher gesperrt." });
      return Promise.resolve(undefined);
    });
    const dialog = await entwuerfeOeffnen(konto, vi.fn());
    const knopf = (text: string) =>
      [...document.querySelectorAll("button")].find((k) => k.textContent === text) as HTMLButtonElement;
    knopf("Löschen").click();
    await warten();
    // Bestätigungsdialog: endgültig löschen
    [...document.querySelectorAll("dialog.bestaetigen button")].find((k) => k.textContent === "Löschen")!.dispatchEvent(new MouseEvent("click"));
    await warten();
    await warten();
    expect(dialog.querySelector(".status")?.textContent).toBe("Speicher gesperrt.");
    expect(dialog.querySelector(".entwurf-betreff")?.textContent).toBe("Alt");
  });
});

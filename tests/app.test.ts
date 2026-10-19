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

/** Antworten des Backends je Befehl. */
function backend(antworten: Record<string, unknown>) {
  invoke.mockImplementation((befehl: string) => {
    const antwort = antworten[befehl];
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

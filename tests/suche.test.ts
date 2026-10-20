import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));

const { App } = await import("../src/app");
const { hervorheben, suchwoerter, SUCH_VERZOEGERUNG } = await import("../src/postfach/suche");

const warten = (ms = 0) => new Promise((r) => setTimeout(r, ms));

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

function mail(uid: number, betreff: string, extra: Record<string, unknown> = {}) {
  return {
    uid,
    betreff,
    absender: "Jürg Müller",
    absender_adresse: "juerg@example.org",
    datum: 1_700_000_000 + uid,
    gelesen: true,
    markiert: false,
    groesse: 100,
    ...extra,
  };
}

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
    kennung: "",
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

async function starten(antworten: Record<string, unknown>) {
  invoke.mockImplementation((befehl: string, argumente: unknown) => {
    const antwort = befehl in antworten ? antworten[befehl] : befehl === "mail_anzeigen" ? einfacheMail : undefined;
    return Promise.resolve(typeof antwort === "function" ? (antwort as (a: unknown) => unknown)(argumente) : antwort);
  });
  const wurzel = document.createElement("div");
  document.body.append(wurzel);
  const app = new App(wurzel);
  await app.starten();
  await warten();
  await warten();
  return wurzel;
}

const normaleListe = {
  mails: [mail(5, "Neueste Mail"), mail(4, "Zweite Mail")],
  auf_server: 2,
  offline: null,
  weitere: false,
};

function eingeben(wurzel: HTMLElement, text: string) {
  const feld = wurzel.querySelector<HTMLInputElement>("input.suchfeld")!;
  feld.value = text;
  feld.dispatchEvent(new Event("input"));
  return feld;
}

function knopf(wurzel: HTMLElement, text: string): HTMLButtonElement {
  const gefunden = [...wurzel.querySelectorAll("button")].find((k) => k.textContent === text);
  expect(gefunden, text).toBeTruthy();
  return gefunden as HTMLButtonElement;
}

const aufrufe = (befehl: string) => invoke.mock.calls.filter(([b]) => b === befehl);

describe("Suchwörter und Hervorheben", () => {
  it("zerlegt den Begriff wie die lokale Suche", () => {
    expect(suchwoerter('  "Rechnung"  März ')).toEqual(["Rechnung", "März"]);
    expect(suchwoerter('" * \\ --')).toEqual([]);
  });

  it("hebt Treffer ohne Rücksicht auf Gross-/Kleinschreibung und Akzente hervor", () => {
    const p = document.createElement("p");
    p.append(...hervorheben("Grüsse von Jürg Müller", "muller GRÜ"));
    expect([...p.querySelectorAll("mark")].map((m) => m.textContent)).toEqual(["Grü", "Müller"]);
    expect(p.textContent).toBe("Grüsse von Jürg Müller");
  });

  it("setzt Text nie als HTML", () => {
    const p = document.createElement("p");
    p.append(...hervorheben("<img src=x onerror=alert(1)> Rechnung", "rechnung"));
    expect(p.querySelector("img")).toBeNull();
    expect(p.querySelector("mark")?.textContent).toBe("Rechnung");
    expect(p.textContent).toBe("<img src=x onerror=alert(1)> Rechnung");
  });

  it("lässt Text ohne Suchwörter unverändert", () => {
    const p = document.createElement("p");
    p.append(...hervorheben("Betreff", '"*'));
    expect(p.querySelector("mark")).toBeNull();
    expect(p.textContent).toBe("Betreff");
  });
});

describe("Suche in der Oberfläche", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  it("sucht nach kurzer Verzögerung nur lokal und hebt Treffer hervor", async () => {
    const wurzel = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: normaleListe,
      mails_suchen: {
        mails: [{ ...mail(3, "Rechnung März"), konto: "k1", ordner: "INBOX" }],
        weitere: false,
      },
    });
    eingeben(wurzel, "Rech");
    eingeben(wurzel, "Rechnung");
    await warten(SUCH_VERZOEGERUNG / 3);
    expect(aufrufe("mails_suchen")).toHaveLength(0);
    await warten(SUCH_VERZOEGERUNG + 50);
    // Nur einmal, mit dem letzten Begriff, nur lokal.
    expect(aufrufe("mails_suchen")).toEqual([
      ["mails_suchen", { konto: "k1", ordner: "INBOX", begriff: "Rechnung", feld: "alle", start: 0, anzahl: 200 }],
    ]);
    expect(aufrufe("mails_server_suchen")).toHaveLength(0);
    const zeilen = wurzel.querySelectorAll("button.mail");
    expect(zeilen).toHaveLength(1);
    expect(zeilen[0].querySelector(".mail-betreff mark")?.textContent).toBe("Rechnung");
    expect(wurzel.querySelector(".such-info")?.textContent).toBe("1 Treffer");
  });

  it("„Suche beenden“ führt zur vorherigen Ansicht zurück", async () => {
    const wurzel = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: normaleListe,
      mails_suchen: { mails: [], weitere: false },
    });
    const feld = eingeben(wurzel, "nichts");
    feld.form!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(wurzel.querySelectorAll("button.mail")).toHaveLength(0);
    expect(wurzel.textContent).toContain("Keine Treffer auf diesem Gerät.");
    knopf(wurzel, "Suche beenden").click();
    const betreffe = [...wurzel.querySelectorAll(".mail-betreff")].map((b) => b.textContent);
    expect(betreffe).toEqual(["Neueste Mail", "Zweite Mail"]);
    expect(feld.value).toBe("");
    expect(knopf(wurzel, "Suche beenden").hidden).toBe(true);
  });

  it("sucht erst nach Klick auf dem Server, mit Feld und Bereich", async () => {
    const wurzel = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: normaleListe,
      mails_suchen: { mails: [], weitere: false },
      mails_server_suchen: {
        mails: [{ ...mail(1, "Alte Offerte"), konto: "k1", ordner: "Archiv" }],
        weitere: false,
      },
    });
    const server = knopf(wurzel, "Auf dem Server suchen");
    expect(server.disabled).toBe(true);
    eingeben(wurzel, "Offerte");
    const feldWahl = wurzel.querySelector<HTMLSelectElement>("select[name=suchfeld]")!;
    const bereich = wurzel.querySelector<HTMLSelectElement>("select[name=suchbereich]")!;
    feldWahl.value = "betreff";
    bereich.value = "konto";
    bereich.dispatchEvent(new Event("change"));
    await warten();
    expect(aufrufe("mails_suchen").at(-1)?.[1]).toEqual({
      konto: "k1",
      ordner: null,
      begriff: "Offerte",
      feld: "betreff",
      start: 0,
      anzahl: 200,
    });
    expect(aufrufe("mails_server_suchen")).toHaveLength(0);
    expect(server.disabled).toBe(false);
    server.click();
    await warten();
    expect(aufrufe("mails_server_suchen")).toEqual([
      ["mails_server_suchen", { konto: "k1", ordner: null, begriff: "Offerte", feld: "betreff" }],
    ]);
    expect(wurzel.querySelector(".such-info")?.textContent).toBe("1 Treffer auf dem Server");
    expect(wurzel.querySelector(".mail-ordner")?.textContent).toBe("Ordner: Archiv");
  });

  it("öffnet und markiert Treffer im richtigen Ordner", async () => {
    const wurzel = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: normaleListe,
      mails_suchen: {
        mails: [{ ...mail(7, "Im Archiv", { gelesen: false }), konto: "k1", ordner: "Archiv" }],
        weitere: false,
      },
    });
    eingeben(wurzel, "archiv").form!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    wurzel.querySelector<HTMLButtonElement>("button.mail")!.click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mail_anzeigen", { konto: "k1", ordner: "Archiv", uid: 7, externLaden: false });
    expect(invoke).toHaveBeenCalledWith("mail_gelesen_setzen", { konto: "k1", ordner: "Archiv", uid: 7, gelesen: true });
    expect(wurzel.querySelector("button.mail")?.classList.contains("ungelesen")).toBe(false);
  });

  it("lädt weitere Treffer nach", async () => {
    const wurzel = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: normaleListe,
      mails_suchen: (a: { start: number }) => ({
        mails: [{ ...mail(100 - a.start, `Treffer ${a.start}`), konto: "k1", ordner: "INBOX" }],
        weitere: a.start === 0,
      }),
    });
    eingeben(wurzel, "treffer").form!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(wurzel.querySelector(".such-info")?.textContent).toBe("1+ Treffer");
    knopf(wurzel, "Mehr laden").click();
    await warten();
    expect(aufrufe("mails_suchen").at(-1)?.[1]).toMatchObject({ start: 1, anzahl: 200 });
    expect(wurzel.querySelectorAll("button.mail")).toHaveLength(2);
  });

  it("Ctrl+F und Cmd+F springen ins Suchfeld", async () => {
    const wurzel = await starten({
      konten_auflisten: [konto],
      ordner_auflisten: ordner,
      mails_auflisten: normaleListe,
    });
    const feld = wurzel.querySelector<HTMLInputElement>("input.suchfeld")!;
    for (const taste of [{ ctrlKey: true }, { metaKey: true }]) {
      (document.activeElement as HTMLElement | null)?.blur();
      const ereignis = new KeyboardEvent("keydown", { key: "f", cancelable: true, bubbles: true, ...taste });
      document.dispatchEvent(ereignis);
      expect(ereignis.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(feld);
    }
  });

  it("zeigt keine Suchleiste ohne Konto", async () => {
    const wurzel = await starten({ konten_auflisten: [] });
    expect(wurzel.querySelector<HTMLFormElement>("form.suchleiste")?.hidden).toBe(true);
  });
});

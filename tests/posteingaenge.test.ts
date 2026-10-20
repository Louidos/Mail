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
const { kontoKennung, kuerzel } = await import("../src/postfach/konten");

const warten = () => new Promise((r) => setTimeout(r, 0));

function konto(id: string, name: string, email: string) {
  return {
    id,
    name,
    email,
    benutzername: email,
    imap: { host: `imap.${id}.example`, port: 993, verschluesselung: "tls" as const },
    smtp: { host: `smtp.${id}.example`, port: 465, verschluesselung: "tls" as const },
    gesendet_ablegen: true,
    absender_kennung: "",
  };
}

const kontoA = konto("ka", "Anna Muster", "anna@a.example");
const kontoB = konto("kb", "Bea Beispiel", "bea@b.example");

const ordner = [
  { name: "INBOX", anzeigename: "Posteingang", trenner: "/", sonder: "posteingang", waehlbar: true },
  { name: "Trash", anzeigename: "Papierkorb", trenner: "/", sonder: "papierkorb", waehlbar: true },
];

function eintrag(kontoId: string, uid: number, betreff: string, datum: number, gelesen = true) {
  return {
    konto: kontoId,
    ordner: "INBOX",
    uid,
    betreff,
    absender: "Chris",
    absender_adresse: "chris@example.org",
    datum,
    gelesen,
    markiert: false,
    groesse: 10,
  };
}

/** Mail von Chris an Konto B (für „Antworten“). */
const mailAnB = {
  betreff: "Frage an B",
  von: [{ name: "Chris", adresse: "chris@example.org" }],
  an: [{ name: null, adresse: "bea@b.example" }],
  cc: [],
  antwort_an: [],
  datum: 1_700_000_200,
  message_id: "<frage@example.org>",
  referenzen: [],
  text: "Hallo Bea",
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
    verfahren: "openpgp",
    verschluesselung: "keine",
    signatur: "keine",
    signatur_teilweise: false,
    signierer: null,
    signierer_schluessel: null,
    meldung: null,
    passphrase_fuer: null,
    hinweise: [],
    zertifikat_uebernehmbar: false,
  },
};

const gemischt = {
  mails: [
    eintrag("kb", 2, "Frage an B", 1_700_000_300, false),
    eintrag("ka", 900, "Neues für A", 1_700_000_200),
    eintrag("kb", 1, "Alt bei B", 1_700_000_100),
  ],
  weitere: true,
  ungelesen: 1,
  hinweis: null,
};

async function starten(antworten: Record<string, unknown>) {
  invoke.mockImplementation((befehl: string, argumente: { entwurf?: object }) => {
    if (befehl === "entwurf_speichern") return Promise.resolve({ ...argumente.entwurf, id: "e1" });
    const antwort = befehl in antworten ? antworten[befehl] : undefined;
    return Promise.resolve(typeof antwort === "function" ? (antwort as (a: unknown) => unknown)(argumente) : antwort);
  });
  const wurzel = document.createElement("div");
  document.body.append(wurzel);
  const app = new App(wurzel);
  await app.starten();
  await warten();
  await warten();
  return { app, wurzel };
}

const zweiKonten = {
  konten_auflisten: [kontoA, kontoB],
  ordner_auflisten: ordner,
  mails_auflisten: { mails: [], auf_server: 0, offline: null, weitere: false },
  posteingaenge_ungelesen: 3,
  alle_posteingaenge: gemischt,
  mail_anzeigen: mailAnB,
};

function knopf(wurzel: ParentNode, text: string): HTMLButtonElement {
  const gefunden = [...wurzel.querySelectorAll("button")].find((k) => k.textContent === text);
  expect(gefunden, text).toBeTruthy();
  return gefunden as HTMLButtonElement;
}

async function alleOeffnen(wurzel: HTMLElement) {
  wurzel.querySelector<HTMLButtonElement>("button.alle-posteingaenge")!.click();
  await warten();
  await warten();
}

describe("Kontokennung", () => {
  it("bildet Kürzel aus dem Namen, sonst aus der Adresse", () => {
    expect(kuerzel(kontoA)).toBe("AM");
    expect(kuerzel({ ...kontoA, name: "Jürg" })).toBe("J");
    expect(kuerzel({ ...kontoA, name: "  " })).toBe("A");
  });

  it("zeigt bei gleichen Namen die Adresse und verteilt die Farben", () => {
    const zweite = { ...kontoB, name: kontoA.name };
    expect(kontoKennung(kontoA, [kontoA, zweite])).toEqual({ kuerzel: "AM", name: "anna@a.example", farbe: 1 });
    expect(kontoKennung(zweite, [kontoA, zweite]).farbe).toBe(2);
    expect(kontoKennung(kontoB, [kontoA, kontoB]).name).toBe("Bea Beispiel");
  });
});

describe("Gemeinsamer Posteingang", () => {
  beforeEach(() => {
    invoke.mockReset();
    ereignisHoerer = null;
    document.body.replaceChildren();
  });

  it("erscheint erst ab zwei Konten, mit Zahl der ungelesenen Mails", async () => {
    const { wurzel: eines } = await starten({ ...zweiKonten, konten_auflisten: [kontoA] });
    expect(eines.querySelector(".alle-posteingaenge")).toBeNull();
    expect(invoke.mock.calls.some((c) => c[0] === "posteingaenge_ungelesen")).toBe(false);
    document.body.replaceChildren();

    const { wurzel } = await starten(zweiKonten);
    const alle = wurzel.querySelector<HTMLButtonElement>("button.alle-posteingaenge")!;
    expect(alle).toBeTruthy();
    // Oberhalb der Konten.
    const bereich = wurzel.querySelector("nav.konten")!;
    expect(bereich.firstElementChild?.contains(alle)).toBe(true);
    expect(alle.querySelector(".zaehler")?.textContent).toBe("3");
    expect(alle.getAttribute("aria-label")).toBe("Alle Posteingänge, 3 ungelesen");
  });

  it("zeigt die Mails aller Posteingänge gemischt, jede mit ihrem Konto – ohne neue Verbindungen", async () => {
    const { wurzel } = await starten(zweiKonten);
    invoke.mockClear();
    await alleOeffnen(wurzel);
    expect(invoke).toHaveBeenCalledWith("alle_posteingaenge", { start: 0, anzahl: 50, nachladen: false });
    // Kein Abgleich und keine Ordnerabfrage beim Öffnen: nur der lokale Speicher.
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(["alle_posteingaenge"]);
    const zeilen = [...wurzel.querySelectorAll("button.mail")];
    expect(zeilen.map((z) => z.querySelector(".mail-betreff")?.textContent)).toEqual([
      "Frage an B",
      "Neues für A",
      "Alt bei B",
    ]);
    const marken = zeilen.map((z) => z.querySelector(".mail-konto")!);
    expect(marken.map((m) => m.textContent)).toEqual(["BBKonto: Bea Beispiel", "AMKonto: Anna Muster", "BBKonto: Bea Beispiel"]);
    // Nie nur Farbe: Kürzel und Name stehen als Text da, die Farbe hängt am Kürzel.
    expect(marken[0].querySelector(".konto-kuerzel")?.getAttribute("data-farbe")).toBe("2");
    expect(marken[1].querySelector(".konto-kuerzel")?.getAttribute("data-farbe")).toBe("1");
    expect(wurzel.querySelector<HTMLButtonElement>("button.alle-posteingaenge")?.getAttribute("aria-current")).toBe("true");

    knopf(wurzel, "Ältere Mails laden").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("alle_posteingaenge", { start: 3, anzahl: 50, nachladen: true });
  });

  it("Aktionen treffen das Konto der Mail", async () => {
    const { wurzel } = await starten({ ...zweiKonten, mail_gelesen_setzen: undefined, mail_loeschen: undefined });
    await alleOeffnen(wurzel);
    wurzel.querySelector<HTMLButtonElement>("button.mail")!.click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mail_anzeigen", { konto: "kb", ordner: "INBOX", uid: 2, externLaden: false });
    expect(invoke).toHaveBeenCalledWith("mail_gelesen_setzen", { konto: "kb", ordner: "INBOX", uid: 2, gelesen: true });
    expect(wurzel.querySelector(".lese-konto")?.textContent).toBe("Bea Beispiel <bea@b.example>");
    knopf(wurzel.querySelector(".lesen")!, "Löschen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mail_loeschen", { konto: "kb", ordner: "INBOX", uid: 2 });
    expect(invoke).not.toHaveBeenCalledWith("mail_loeschen", expect.objectContaining({ konto: "ka" }));
    expect(wurzel.querySelectorAll("button.mail")).toHaveLength(2);
  });

  it("antwortet über das Konto der Mail, auch wenn ein anderes das Standardkonto ist", async () => {
    const { wurzel } = await starten({ ...zweiKonten, mail_senden: { abgelegt: true, hinweis: null } });
    await alleOeffnen(wurzel);
    wurzel.querySelector<HTMLButtonElement>("button.mail")!.click();
    await warten();
    knopf(wurzel, "Antworten").click();
    await warten();
    const dialog = document.querySelector("dialog")!;
    // Das Absenderkonto ist im Verfassen-Fenster sichtbar.
    const von = dialog.querySelector<HTMLSelectElement>("select[name=konto]")!;
    expect(von.value).toBe("kb");
    expect(von.selectedOptions[0].textContent).toBe("Bea Beispiel <bea@b.example>");
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    await warten();
    const senden = invoke.mock.calls.find((c) => c[0] === "mail_senden")!;
    expect(senden[1].entwurf).toMatchObject({ konto: "kb", an: "Chris <chris@example.org>", betreff: "Re: Frage an B" });
  });

  it("leitet über das Konto der Mail weiter", async () => {
    const { wurzel } = await starten(zweiKonten);
    await alleOeffnen(wurzel);
    wurzel.querySelector<HTMLButtonElement>("button.mail")!.click();
    await warten();
    knopf(wurzel, "Weiterleiten").click();
    await warten();
    expect(document.querySelector<HTMLSelectElement>("dialog select[name=konto]")?.value).toBe("kb");
  });

  it("„Neue Mail“ nimmt das Standardkonto, die Wahl bleibt offen", async () => {
    const { wurzel } = await starten(zweiKonten);
    await alleOeffnen(wurzel);
    knopf(wurzel, "Neue Mail").click();
    const von = document.querySelector<HTMLSelectElement>("dialog select[name=konto]")!;
    expect(von.value).toBe("ka");
    expect(von.disabled).toBe(false);
    expect([...von.options].map((o) => o.value)).toEqual(["ka", "kb"]);
  });

  it("aktualisiert sich bei neuen Mails eines beliebigen Kontos", async () => {
    const { wurzel } = await starten(zweiKonten);
    await alleOeffnen(wurzel);
    invoke.mockClear();
    ereignisHoerer?.({ payload: { art: "mails_geaendert", konto: "kb", ordner: "INBOX" } });
    await warten();
    expect(invoke).toHaveBeenCalledWith("alle_posteingaenge", { start: 0, anzahl: 50, nachladen: false });
    expect(invoke.mock.calls.some((c) => c[0] === "posteingaenge_ungelesen")).toBe(true);
  });

  it("durchsucht die Posteingänge aller Konten", async () => {
    const { wurzel } = await starten({
      ...zweiKonten,
      mails_suchen: { mails: [eintrag("kb", 2, "Frage an B", 1)], weitere: false },
      mails_server_suchen: { mails: [], weitere: false },
    });
    await alleOeffnen(wurzel);
    const feld = wurzel.querySelector<HTMLInputElement>("input.suchfeld")!;
    expect(wurzel.querySelector<HTMLSelectElement>("select[name=suchbereich]")?.hidden).toBe(true);
    feld.value = "frage";
    feld.form!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(invoke).toHaveBeenCalledWith("mails_suchen", {
      konto: null,
      ordner: null,
      begriff: "frage",
      feld: "alle",
      start: 0,
      anzahl: 200,
    });
    expect(wurzel.querySelector("button.mail .mail-konto")?.textContent).toContain("Bea Beispiel");
    knopf(wurzel, "Auf dem Server suchen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("mails_server_suchen", { konto: null, ordner: null, begriff: "frage", feld: "alle" });
  });

  it("springt zurück zu einem Konto, wenn nur noch eines übrig ist", async () => {
    let konten = [kontoA, kontoB];
    const { wurzel } = await starten({ ...zweiKonten, konten_auflisten: () => konten, konto_loeschen: undefined });
    await alleOeffnen(wurzel);
    konten = [kontoA];
    const loeschen = [...wurzel.querySelectorAll<HTMLButtonElement>(".konto button")].filter((k) => k.textContent === "Löschen")[1];
    loeschen.click();
    await warten();
    const ja = [...document.querySelectorAll("dialog button")].find((k) => k.textContent === "Konto löschen") as HTMLButtonElement;
    ja.click();
    for (let i = 0; i < 4; i++) await warten();
    expect(wurzel.querySelector(".alle-posteingaenge")).toBeNull();
    expect(invoke).toHaveBeenCalledWith("mails_auflisten", { konto: "ka", ordner: "INBOX", start: 0, anzahl: 50 });
  });
});

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Konto, ZertifikatInfo, ZertifikatUebersicht } from "../src/api";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { schluesselOeffnen } = await import("../src/schluessel/dialog");

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

const zertifikat = (name: string, adresse: string, fingerabdruck: string): ZertifikatInfo => ({
  fingerabdruck,
  name,
  adressen: [adresse],
  aussteller: "SicherMail Test-Zwischenstelle",
  gueltig_ab: 1_700_000_000,
  gueltig_bis: 4_000_000_000,
  abgelaufen: false,
});

const wurzel = { ...zertifikat("SicherMail Test-Wurzel", "", "AAAA BBBB CCCC"), adressen: [] };

function backend(uebersicht: ZertifikatUebersicht, antworten: Record<string, unknown> = {}) {
  invoke.mockImplementation((befehl: string) => {
    if (befehl === "pgp_uebersicht") return Promise.resolve({ eigene: [], fremde: [] });
    if (befehl === "smime_uebersicht") return Promise.resolve(uebersicht);
    if (befehl in antworten) {
      const antwort = antworten[befehl];
      return antwort instanceof Error ? Promise.reject(antwort) : Promise.resolve(antwort);
    }
    return Promise.resolve(undefined);
  });
}

function knopf(wurzel: ParentNode, text: string): HTMLButtonElement {
  return [...wurzel.querySelectorAll("button")].find((k) => k.textContent === text) as HTMLButtonElement;
}

const leer: ZertifikatUebersicht = { eigene: [], fremde: [], wurzeln: [] };

describe("S/MIME-Zertifikate", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  it("zeigt den Bereich neben OpenPGP", async () => {
    backend(leer);
    const dialog = await schluesselOeffnen([konto]);
    expect(dialog.querySelector("h2")?.textContent).toBe("Schlüssel und Zertifikate");
    const bereich = dialog.querySelector(".smime-bereich") as HTMLElement;
    expect(bereich.textContent).toContain("S/MIME-Zertifikate");
    expect(bereich.textContent).toContain("Noch kein eigenes S/MIME-Zertifikat.");
    expect(bereich.textContent).toContain("Keine – es gelten nur die Stellen im Zertifikatsspeicher des Systems.");
  });

  it("importiert ein eigenes Zertifikat nur mit Passwort", async () => {
    backend(leer, { smime_eigenes_importieren: zertifikat("Anna Test", "anna@example.org", "1111") });
    const dialog = await schluesselOeffnen([konto]);
    const bereich = dialog.querySelector(".smime-bereich") as HTMLElement;
    knopf(bereich, "Eigenes Zertifikat importieren …").click();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("smime_eigenes_importieren", expect.anything());
    expect(dialog.querySelector(".status")?.textContent).toContain("Passwort der .p12/.pfx-Datei");

    const passwort = bereich.querySelector("[name=smime_passwort]") as HTMLInputElement;
    passwort.value = "test-passwort";
    knopf(bereich, "Eigenes Zertifikat importieren …").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("smime_eigenes_importieren", { konto: "k1", passwort: "test-passwort" });
    expect(passwort.value).toBe("");
    expect(dialog.querySelector(".status")?.textContent).toContain("anna@example.org wurde importiert");
  });

  it("zeigt eine Ablehnung beim Import deutlich", async () => {
    backend(leer, {
      smime_eigenes_importieren: Object.assign(new Error("x"), {
        meldung: "Das Zertifikat passt nicht zu diesem Konto: Es gilt für eve@example.org, nicht für anna@example.org.",
      }),
    });
    const dialog = await schluesselOeffnen([konto]);
    (dialog.querySelector("[name=smime_passwort]") as HTMLInputElement).value = "test-passwort";
    knopf(dialog, "Eigenes Zertifikat importieren …").click();
    await warten();
    const status = dialog.querySelector(".status") as HTMLElement;
    expect(status.dataset.art).toBe("fehler");
    expect(status.textContent).toContain("passt nicht zu diesem Konto");
  });

  it("listet eigene Zertifikate mit Aussteller, Ablauf und Fingerabdruck und entsperrt sie", async () => {
    const eigenes = { ...zertifikat("Anna Test", "anna@example.org", "1111 2222"), konto: "k1", entsperrt: false };
    backend({ ...leer, eigene: [eigenes], fremde: [zertifikat("Bea Test", "bea@example.org", "3333")] });
    const dialog = await schluesselOeffnen([konto]);
    const bereich = dialog.querySelector(".smime-bereich") as HTMLElement;
    expect(bereich.textContent).toContain("Ausgestellt von: SicherMail Test-Zwischenstelle");
    expect(bereich.textContent).toContain("Gültig bis");
    expect(bereich.textContent).toContain("1111 2222");
    expect(bereich.textContent).toContain("Bea Test <bea@example.org>");
    (bereich.querySelector("[name=smime_entsperren]") as HTMLInputElement).value = "test-passwort";
    knopf(bereich, "Entsperren").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("smime_entsperren", { fingerabdruck: "1111 2222", passphrase: "test-passwort" });
  });

  it("übernimmt eine Wurzel erst nach Bestätigung des Fingerabdrucks", async () => {
    backend(leer, { smime_wurzel_lesen: wurzel, smime_wurzel_bestaetigen: wurzel });
    const dialog = await schluesselOeffnen([konto]);
    knopf(dialog, "Wurzelzertifikat übernehmen …").click();
    await warten();
    let frage = document.querySelector("dialog.bestaetigen") as HTMLElement;
    expect(frage.textContent).toContain("AAAA BBBB CCCC");
    expect(frage.textContent).toContain("SicherMail Test-Wurzel");
    knopf(frage, "Abbrechen").click();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("smime_wurzel_bestaetigen", expect.anything());

    knopf(dialog, "Wurzelzertifikat übernehmen …").click();
    await warten();
    frage = document.querySelector("dialog.bestaetigen") as HTMLElement;
    knopf(frage, "Wurzel übernehmen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("smime_wurzel_bestaetigen", { fingerabdruck: "AAAA BBBB CCCC" });
  });

  it("entfernt übernommene Wurzeln nach Rückfrage", async () => {
    backend({ ...leer, wurzeln: [wurzel] });
    const dialog = await schluesselOeffnen([konto]);
    knopf(dialog.querySelector(".smime-bereich")!, "Entfernen").click();
    await warten();
    knopf(document.querySelector("dialog.bestaetigen")!, "Entfernen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("smime_loeschen", { fingerabdruck: "AAAA BBBB CCCC" });
  });
});

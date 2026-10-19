import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Konto, SchluesselUebersicht } from "../src/api";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { schluesselOeffnen, passphrasePruefen } = await import("../src/schluessel/dialog");

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

const bea = {
  fingerabdruck: "BBBB 2222",
  benutzer_ids: ["Bea <bea@example.org>"],
  adressen: ["bea@example.org"],
  erstellt: 1_700_000_000,
  kann_verschluesseln: true,
};

function backend(uebersicht: SchluesselUebersicht) {
  invoke.mockImplementation((befehl: string) => {
    if (befehl === "pgp_uebersicht") return Promise.resolve(uebersicht);
    if (befehl === "pgp_exportieren") return Promise.resolve("anna@example.org.asc");
    return Promise.resolve(undefined);
  });
}

function knopf(wurzel: ParentNode, text: string): HTMLButtonElement {
  return [...wurzel.querySelectorAll("button")].find((k) => k.textContent === text) as HTMLButtonElement;
}

describe("Schlüssel (OpenPGP)", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  it("prüft die Passphrase für neue Schlüssel", () => {
    expect(passphrasePruefen("kurz", "kurz")).toMatch(/mindestens 8/);
    expect(passphrasePruefen("lange-passphrase", "andere-passphrase")).toMatch(/stimmen nicht/);
    expect(passphrasePruefen("lange-passphrase", "lange-passphrase")).toBeNull();
  });

  it("erzeugt einen Schlüssel erst mit gültiger Passphrase", async () => {
    backend({ eigene: [], fremde: [] });
    const dialog = await schluesselOeffnen([konto]);
    expect(dialog.textContent).toContain("Noch kein eigener Schlüssel");
    const [passphrase, wiederholung] = [...dialog.querySelectorAll("input[type=password]")] as HTMLInputElement[];
    passphrase.value = "kurz";
    wiederholung.value = "kurz";
    knopf(dialog, "Schlüssel erzeugen").click();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("pgp_erzeugen", expect.anything());
    expect(dialog.querySelector(".status")?.textContent).toMatch(/mindestens 8/);

    passphrase.value = "lange-passphrase";
    wiederholung.value = "lange-passphrase";
    knopf(dialog, "Schlüssel erzeugen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_erzeugen", { konto: "k1", passphrase: "lange-passphrase" });
    expect(passphrase.value).toBe("");
  });

  it("importiert den eigenen Schlüssel nur mit Passphrase", async () => {
    backend({ eigene: [], fremde: [] });
    const dialog = await schluesselOeffnen([konto]);
    knopf(dialog, "Eigenen Schlüssel importieren …").click();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("pgp_eigenen_importieren", expect.anything());
    (dialog.querySelector("input[name=passphrase]") as HTMLInputElement).value = "test-passphrase";
    knopf(dialog, "Eigenen Schlüssel importieren …").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_eigenen_importieren", { konto: "k1", passphrase: "test-passphrase" });
  });

  it("zeigt Fingerabdrücke, exportiert und löscht erst nach Rückfrage", async () => {
    backend({
      eigene: [{ ...bea, fingerabdruck: "AAAA 1111", benutzer_ids: ["Anna <anna@example.org>"], konto: "k1", geschuetzt: true, entsperrt: false }],
      fremde: [bea],
    });
    const dialog = await schluesselOeffnen([konto]);
    expect(dialog.textContent).toContain("AAAA 1111");
    expect(dialog.textContent).toContain("BBBB 2222");
    expect(dialog.textContent).toContain("Mit Passphrase geschützt.");

    knopf(dialog, "Öffentlichen Schlüssel exportieren …").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_exportieren", { fingerabdruck: "AAAA 1111" });
    expect(dialog.querySelector(".status")?.textContent).toContain("anna@example.org.asc");

    const loeschen = dialog.querySelectorAll("button.gefahr");
    (loeschen[0] as HTMLButtonElement).click();
    await warten();
    const frage = document.querySelector("dialog.bestaetigen")!;
    expect(frage.textContent).toContain("endgültig");
    knopf(frage, "Abbrechen").click();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("pgp_loeschen", expect.anything());

    (loeschen[1] as HTMLButtonElement).click();
    await warten();
    knopf(document.querySelector("dialog.bestaetigen")!, "Endgültig löschen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_loeschen", { fingerabdruck: "BBBB 2222" });
  });

  it("importiert öffentliche Schlüssel anderer über den Backend-Dialog", async () => {
    backend({ eigene: [], fremde: [] });
    const dialog = await schluesselOeffnen([konto]);
    knopf(dialog, "Öffentlichen Schlüssel importieren …").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("pgp_oeffentliche_importieren");
  });
});

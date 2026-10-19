import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { kontoDialogOeffnen, pruefergebnisText } = await import("../src/konto/dialog");

const warten = () => new Promise((r) => setTimeout(r, 0));

function ausfuellen(dialog: HTMLDialogElement) {
  const setze = (name: string, wert: string) => {
    (dialog.querySelector(`[name=${name}]`) as HTMLInputElement).value = wert;
  };
  setze("name", "Anna");
  setze("email", "anna@example.org");
  setze("benutzername", "anna");
  setze("passwort", "geheim");
  setze("imap_host", "imap.example.org");
  setze("smtp_host", "smtp.example.org");
}

describe("Kontodialog", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  it("schlägt beim Wechsel auf STARTTLS den passenden Port vor", () => {
    const dialog = kontoDialogOeffnen(null, () => {});
    const auswahl = dialog.querySelector("[name=imap_verschluesselung]") as HTMLSelectElement;
    const port = dialog.querySelector("[name=imap_port]") as HTMLInputElement;
    expect(port.value).toBe("993");
    auswahl.value = "starttls";
    auswahl.dispatchEvent(new Event("change"));
    expect(port.value).toBe("143");
  });

  it("bietet keine unverschlüsselte Verbindung an", () => {
    const dialog = kontoDialogOeffnen(null, () => {});
    const werte = [...dialog.querySelectorAll("select option")].map((o) => (o as HTMLOptionElement).value);
    expect(new Set(werte)).toEqual(new Set(["tls", "starttls"]));
  });

  it("speichert erst nach erfolgreicher Prüfung im Backend und schliesst dann", async () => {
    const gespeichert = vi.fn();
    invoke.mockResolvedValue({ id: "neu" });
    const dialog = kontoDialogOeffnen(null, gespeichert);
    ausfuellen(dialog);
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(invoke).toHaveBeenCalledWith("konto_speichern", expect.objectContaining({ eingabe: expect.anything() }));
    expect(gespeichert).toHaveBeenCalledWith({ id: "neu" });
    expect(document.body.contains(dialog)).toBe(false);
  });

  it("zeigt den Fehler des Backends und bleibt offen", async () => {
    const gespeichert = vi.fn();
    invoke.mockRejectedValue({
      art: "anmeldung_falsch",
      meldung: "IMAP (Empfang): Anmeldung fehlgeschlagen. Bitte Benutzername und Passwort prüfen.",
    });
    const dialog = kontoDialogOeffnen(null, gespeichert);
    ausfuellen(dialog);
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(gespeichert).not.toHaveBeenCalled();
    expect(dialog.querySelector(".status")?.textContent).toContain("Nicht gespeichert");
    expect(dialog.querySelector(".status")?.textContent).toContain("Anmeldung fehlgeschlagen");
    expect(document.body.contains(dialog)).toBe(true);
  });

  it("sendet ohne Pflichtangaben nichts ans Backend", async () => {
    const dialog = kontoDialogOeffnen(null, () => {});
    dialog.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await warten();
    expect(invoke).not.toHaveBeenCalled();
    expect(dialog.querySelector(".status")?.textContent).toMatch(/Namen/);
  });

  it("fasst das Testergebnis verständlich zusammen", () => {
    expect(pruefergebnisText({ imap: null, smtp: null }).erfolg).toBe(true);
    const ergebnis = pruefergebnisText({ imap: { art: "kein_starttls", meldung: "Kein STARTTLS." }, smtp: null });
    expect(ergebnis.erfolg).toBe(false);
    expect(ergebnis.text).toContain("IMAP (Empfang): Kein STARTTLS.");
    expect(ergebnis.text).toContain("SMTP (Versand): Verbindung in Ordnung.");
  });
});

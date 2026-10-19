import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { einstellungenOeffnen } = await import("../src/einstellungen/dialog");

const warten = () => new Promise((r) => setTimeout(r, 0));

function backend(einstellungen: { lokal_speichern: boolean; externe_inhalte_erlaubt: string[]; speicher_hinweis: string | null }) {
  invoke.mockImplementation((befehl: string) => {
    if (befehl === "einstellungen_laden") return Promise.resolve(einstellungen);
    return Promise.resolve(undefined);
  });
}

function knopf(wurzel: ParentNode, text: string): HTMLButtonElement {
  return [...wurzel.querySelectorAll("button")].find((k) => k.textContent === text) as HTMLButtonElement;
}

describe("Einstellungen", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  it("zeigt „Mails lokal speichern“ standardmässig eingeschaltet", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null });
    const dialog = await einstellungenOeffnen(() => {});
    expect((dialog.querySelector("[name=lokal_speichern]") as HTMLInputElement).checked).toBe(true);
    expect(dialog.textContent).toContain("Keine Absender freigegeben.");
  });

  it("leert den lokalen Speicher erst nach Bestätigung", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null });
    const geaendert = vi.fn();
    const dialog = await einstellungenOeffnen(geaendert);
    knopf(dialog, "Lokalen Speicher leeren").click();
    await warten();
    expect(invoke).not.toHaveBeenCalledWith("lokalen_speicher_leeren");
    const frage = document.querySelector("dialog.bestaetigen")!;
    knopf(frage, "Speicher leeren").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("lokalen_speicher_leeren");
    expect(geaendert).toHaveBeenCalled();
    expect(dialog.querySelector(".status")?.textContent).toContain("geleert");
  });

  it("schaltet die lokale Speicherung nur nach Rückfrage aus", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null });
    const dialog = await einstellungenOeffnen(() => {});
    const schalter = dialog.querySelector("[name=lokal_speichern]") as HTMLInputElement;
    schalter.checked = false;
    schalter.dispatchEvent(new Event("change"));
    await warten();
    knopf(document.querySelector("dialog.bestaetigen")!, "Abbrechen").click();
    await warten();
    expect(schalter.checked).toBe(true);
    expect(invoke).not.toHaveBeenCalledWith("lokal_speichern_setzen", expect.anything());

    schalter.checked = false;
    schalter.dispatchEvent(new Event("change"));
    await warten();
    knopf(document.querySelector("dialog.bestaetigen")!, "Ausschalten und löschen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("lokal_speichern_setzen", { an: false });
  });

  it("listet freigegebene Absender und kann sie entfernen", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: ["bea@example.org"], speicher_hinweis: "Hinweis" });
    const dialog = await einstellungenOeffnen(() => {});
    expect(dialog.querySelector(".absender-liste")?.textContent).toContain("bea@example.org");
    expect(dialog.querySelector(".warnung")?.textContent).toBe("Hinweis");
    knopf(dialog, "Entfernen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("absender_extern_entfernen", { adresse: "bea@example.org" });
  });
});

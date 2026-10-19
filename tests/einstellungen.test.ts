import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { einstellungenOeffnen } = await import("../src/einstellungen/dialog");

const warten = () => new Promise((r) => setTimeout(r, 0));

interface TestEinstellungen {
  lokal_speichern: boolean;
  externe_inhalte_erlaubt: string[];
  speicher_hinweis: string | null;
  proxy_externe_bilder?: boolean;
  proxy_adresse?: string;
  darstellung?: "automatisch" | "hell" | "dunkel";
}

function backend(einstellungen: TestEinstellungen, proxyFehler: string | null = null, darstellungFehler = false) {
  invoke.mockImplementation((befehl: string, argumente?: { an?: boolean }) => {
    if (befehl === "einstellungen_laden") {
      return Promise.resolve({ proxy_externe_bilder: false, proxy_adresse: "", darstellung: "automatisch", ...einstellungen });
    }
    if (befehl === "darstellung_setzen" && darstellungFehler) {
      return Promise.reject({ art: "datei", meldung: "Die Einstellungen konnten nicht gespeichert werden." });
    }
    if (befehl === "proxy_setzen" && argumente?.an && proxyFehler) {
      return Promise.reject({ art: "eingabe", meldung: proxyFehler });
    }
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
    delete document.documentElement.dataset.darstellung;
  });

  it("bietet die Darstellung an, standardmässig „Automatisch“", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null });
    const dialog = await einstellungenOeffnen(() => {});
    const auswahl = dialog.querySelector("[name=darstellung]") as HTMLSelectElement;
    expect(auswahl.value).toBe("automatisch");
    expect([...auswahl.options].map((o) => o.value)).toEqual(["automatisch", "hell", "dunkel"]);
    expect(dialog.querySelector("label[for=einstellung-darstellung]")?.textContent).toBe("Hell oder dunkel");
  });

  it("zeigt die gespeicherte Darstellung und wendet eine neue Wahl sofort an", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null, darstellung: "hell" });
    const dialog = await einstellungenOeffnen(() => {});
    const auswahl = dialog.querySelector("[name=darstellung]") as HTMLSelectElement;
    expect(auswahl.value).toBe("hell");
    auswahl.value = "dunkel";
    auswahl.dispatchEvent(new Event("change"));
    // Sofort sichtbar, noch bevor das Backend geantwortet hat.
    expect(document.documentElement.dataset.darstellung).toBe("dunkel");
    expect(document.documentElement.dataset.aktiv).toBe("dunkel");
    await warten();
    expect(invoke).toHaveBeenCalledWith("darstellung_setzen", { darstellung: "dunkel" });
    expect(dialog.querySelector(".status")?.getAttribute("data-art")).toBe("erfolg");

    auswahl.value = "automatisch";
    auswahl.dispatchEvent(new Event("change"));
    await warten();
    expect(document.documentElement.dataset.darstellung).toBeUndefined();
    expect(invoke).toHaveBeenCalledWith("darstellung_setzen", { darstellung: "automatisch" });
  });

  it("stellt die Darstellung zurück, wenn das Speichern scheitert", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null, darstellung: "hell" }, null, true);
    const dialog = await einstellungenOeffnen(() => {});
    const auswahl = dialog.querySelector("[name=darstellung]") as HTMLSelectElement;
    auswahl.value = "dunkel";
    auswahl.dispatchEvent(new Event("change"));
    await warten();
    expect(auswahl.value).toBe("hell");
    expect(document.documentElement.dataset.darstellung).toBe("hell");
    expect(dialog.querySelector(".status")?.getAttribute("data-art")).toBe("fehler");
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

  it("hat den Proxy für externe Bilder standardmässig aus und zeigt die Einschränkung", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null });
    const dialog = await einstellungenOeffnen(() => {});
    const schalter = dialog.querySelector("[name=proxy_externe_bilder]") as HTMLInputElement;
    const adresse = dialog.querySelector("[name=proxy_adresse]") as HTMLInputElement;
    expect(schalter.checked).toBe(false);
    expect(adresse.disabled).toBe(true);
    expect(knopf(dialog, "Proxy übernehmen").disabled).toBe(true);
    expect(dialog.textContent).toContain("Proxy für externe Bilder verwenden");
    expect(dialog.textContent).toContain(
      "Der Proxy löst Adressen selbst auf. SicherMail kann dann nicht mehr vollständig verhindern, dass Bilder aus deinem eigenen Netz geladen werden.",
    );
  });

  it("speichert den Proxy erst mit Adresse und schaltet ihn wieder aus", async () => {
    backend({ lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null });
    const dialog = await einstellungenOeffnen(() => {});
    const schalter = dialog.querySelector("[name=proxy_externe_bilder]") as HTMLInputElement;
    const adresse = dialog.querySelector("[name=proxy_adresse]") as HTMLInputElement;
    schalter.checked = true;
    schalter.dispatchEvent(new Event("change"));
    await warten();
    expect(adresse.disabled).toBe(false);
    expect(invoke).not.toHaveBeenCalledWith("proxy_setzen", expect.anything());

    adresse.value = " http://proxy.example.org:3128 ";
    knopf(dialog, "Proxy übernehmen").click();
    await warten();
    expect(invoke).toHaveBeenCalledWith("proxy_setzen", { an: true, adresse: "http://proxy.example.org:3128" });
    expect(dialog.querySelector(".status")?.getAttribute("data-art")).toBe("erfolg");

    schalter.checked = false;
    schalter.dispatchEvent(new Event("change"));
    await warten();
    expect(adresse.disabled).toBe(true);
    expect(invoke).toHaveBeenCalledWith("proxy_setzen", { an: false, adresse: "http://proxy.example.org:3128" });
  });

  it("meldet eine ungültige Proxy-Adresse deutlich", async () => {
    backend(
      { lokal_speichern: true, externe_inhalte_erlaubt: [], speicher_hinweis: null, proxy_externe_bilder: true, proxy_adresse: "" },
      "Ungültige Proxy-Adresse.",
    );
    const dialog = await einstellungenOeffnen(() => {});
    const adresse = dialog.querySelector("[name=proxy_adresse]") as HTMLInputElement;
    expect(adresse.disabled).toBe(false);
    adresse.value = "http://anna:geheim@proxy.example.org:3128";
    knopf(dialog, "Proxy übernehmen").click();
    await warten();
    const status = dialog.querySelector(".status")!;
    expect(status.getAttribute("data-art")).toBe("fehler");
    expect(status.textContent).toContain("Ungültige Proxy-Adresse.");
    expect(status.textContent).toContain("Externe Bilder bleiben blockiert.");
  });
});

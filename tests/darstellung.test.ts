import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { aktiveDarstellung, darstellungAnwenden, darstellungStarten, gewaehlteDarstellung } = await import(
  "../src/einstellungen/darstellung"
);

/** Ersetzt `matchMedia`, damit sich ein Wechsel der Systemeinstellung nachstellen lässt. */
function systemNachstellen(dunkel: boolean) {
  const hoerer: (() => void)[] = [];
  const abfrage = {
    get matches() {
      return zustand.dunkel;
    },
    addEventListener: (_art: string, f: () => void) => hoerer.push(f),
  };
  const zustand = { dunkel };
  vi.stubGlobal("matchMedia", (anfrage: string) => {
    expect(anfrage).toBe("(prefers-color-scheme: dark)");
    return abfrage;
  });
  return {
    wechseln(neu: boolean) {
      zustand.dunkel = neu;
      for (const f of hoerer) f();
    },
  };
}

describe("Darstellung", () => {
  beforeEach(() => {
    invoke.mockReset();
    delete document.documentElement.dataset.darstellung;
    delete document.documentElement.dataset.aktiv;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("erzwingt Hell oder Dunkel und lässt bei Automatisch das System entscheiden", () => {
    systemNachstellen(true);
    darstellungAnwenden("hell");
    expect(document.documentElement.dataset.darstellung).toBe("hell");
    expect(aktiveDarstellung()).toBe("hell");
    darstellungAnwenden("dunkel");
    expect(document.documentElement.dataset.darstellung).toBe("dunkel");
    darstellungAnwenden("automatisch");
    expect(document.documentElement.dataset.darstellung).toBeUndefined();
    expect(document.documentElement.dataset.aktiv).toBe("dunkel");
  });

  it("lädt beim Start die gespeicherte Wahl und folgt danach dem System", async () => {
    const system = systemNachstellen(false);
    invoke.mockResolvedValue({ darstellung: "automatisch" });
    await darstellungStarten();
    expect(invoke).toHaveBeenCalledWith("einstellungen_laden");
    expect(gewaehlteDarstellung()).toBe("automatisch");
    expect(document.documentElement.dataset.aktiv).toBe("hell");

    system.wechseln(true);
    expect(document.documentElement.dataset.aktiv).toBe("dunkel");
    expect(document.documentElement.dataset.darstellung).toBeUndefined();

    // Von Hand gewählt: ein Systemwechsel ändert nichts.
    darstellungAnwenden("hell");
    system.wechseln(false);
    system.wechseln(true);
    expect(document.documentElement.dataset.darstellung).toBe("hell");
    expect(document.documentElement.dataset.aktiv).toBe("hell");
  });

  it("übernimmt die vom Backend vor dem Zeichnen gesetzte Wahl, auch wenn das Laden scheitert", async () => {
    systemNachstellen(false);
    document.documentElement.dataset.darstellung = "dunkel";
    invoke.mockRejectedValue(new Error("kein Tauri"));
    await darstellungStarten();
    expect(gewaehlteDarstellung()).toBe("dunkel");
    expect(document.documentElement.dataset.darstellung).toBe("dunkel");
  });
});

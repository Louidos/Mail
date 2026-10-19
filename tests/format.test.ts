import { describe, expect, it } from "vitest";
import { datumKurz, groesse, ordnerKurzname, ordnerTiefe } from "../src/postfach/format";

describe("Formatierung", () => {
  it("zeigt heute nur die Uhrzeit, sonst das Datum", () => {
    const jetzt = new Date(2026, 9, 19, 15, 0);
    const heute = new Date(2026, 9, 19, 9, 5).getTime() / 1000;
    const diesesJahr = new Date(2026, 2, 3, 9, 5).getTime() / 1000;
    const frueher = new Date(2024, 0, 2, 9, 5).getTime() / 1000;
    expect(datumKurz(heute, jetzt)).toBe("09:05");
    expect(datumKurz(diesesJahr, jetzt)).toMatch(/3\./);
    expect(datumKurz(frueher, jetzt)).toBe("02.01.2024");
    expect(datumKurz(null, jetzt)).toBe("");
  });

  it("formatiert Grössen", () => {
    expect(groesse(500)).toBe("500 B");
    expect(groesse(2048)).toBe("2 KB");
    expect(groesse(3 * 1024 * 1024 + 200_000)).toBe("3,2 MB");
  });

  it("erkennt die Tiefe von Ordnern", () => {
    expect(ordnerTiefe("INBOX", "/")).toBe(0);
    expect(ordnerTiefe("Projekte/2026/Kunde", "/")).toBe(2);
    expect(ordnerKurzname("Projekte/2026/Kunde", "/")).toBe("Kunde");
    expect(ordnerKurzname("Archiv", null)).toBe("Archiv");
  });
});

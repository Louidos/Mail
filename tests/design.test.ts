import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(__dirname, "..", "src");
const FARBWERT = /#[0-9a-f]{3,8}\b|\brgba?\(|\bhsla?\(/i;

function dateien(ordner: string): string[] {
  return readdirSync(ordner).flatMap((name) => {
    const pfad = join(ordner, name);
    return statSync(pfad).isDirectory() ? dateien(pfad) : [pfad];
  });
}

/** Der Block mit den zentralen Variablen ganz oben in styles.css. */
function zentralerBlock(css: string): { block: string; rest: string } {
  const ende = css.indexOf("\n}\n") + 3;
  return { block: css.slice(0, ende), rest: css.slice(ende) };
}

describe("Design", () => {
  const css = readFileSync(join(SRC, "styles.css"), "utf8");
  const { block, rest } = zentralerBlock(css);

  it("legt die Farben zentral fest", () => {
    expect(block).toMatch(/^\s*:root,\s*::backdrop\s*\{/m);
    for (const farbe of ["#2563eb", "#1d4ed8", "#eff6ff", "#dbeafe", "#ffffff", "#1f2937", "#6b7280", "#e5e7eb"]) {
      expect(block.toLowerCase()).toContain(farbe);
    }
  });

  it("verwendet ausserhalb der zentralen Definition keine festen Farbwerte", () => {
    const funde = rest.split("\n").filter((zeile) => FARBWERT.test(zeile));
    expect(funde).toEqual([]);
    for (const datei of dateien(SRC).filter((d) => /\.(ts|html)$/.test(d))) {
      const zeilen = readFileSync(datei, "utf8")
        .split("\n")
        .filter((zeile) => FARBWERT.test(zeile));
      expect(zeilen, datei).toEqual([]);
    }
  });

  it("nutzt die vorgegebenen Radien", () => {
    expect(block).toContain("--radius-klein: 8px;");
    expect(block).toContain("--radius-gross: 12px;");
    expect(rest).not.toMatch(/border-radius:\s*\d/);
  });

  it("zeigt den Tastaturfokus sichtbar in Blau", () => {
    expect(rest).toMatch(/:focus-visible\s*\{\s*outline: 2px solid var\(--blau\)/);
  });
});

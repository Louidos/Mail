import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(__dirname, "..", "src");
const FARBWERT = /#[0-9a-f]{3,8}\b|\brgba?\(|\bhsla?\(/i;
/** Markiert in styles.css das Ende des zentralen Bereichs mit allen Designvariablen. */
const ENDE_MARKE = "/* Ende der zentralen Designvariablen */";

function dateien(ordner: string): string[] {
  return readdirSync(ordner).flatMap((name) => {
    const pfad = join(ordner, name);
    return statSync(pfad).isDirectory() ? dateien(pfad) : [pfad];
  });
}

/** Der Bereich mit den zentralen Variablen ganz oben in styles.css (hell und dunkel). */
function zentralerBlock(css: string): { block: string; rest: string } {
  const ende = css.indexOf(ENDE_MARKE);
  expect(ende, "Marke für das Ende des zentralen Bereichs fehlt").toBeGreaterThan(0);
  return { block: css.slice(0, ende), rest: css.slice(ende + ENDE_MARKE.length) };
}

/** Inhalt des Regelblocks, der direkt auf `selektor` folgt (ohne verschachtelte Blöcke). */
function regelblock(css: string, selektor: RegExp): string {
  const treffer = selektor.exec(css);
  expect(treffer, String(selektor)).not.toBeNull();
  const start = css.indexOf("{", treffer!.index) + 1;
  return css.slice(start, css.indexOf("}", start));
}

/** Variablen eines Blocks, deren Wert eine Farbe enthält. */
function farbvariablen(block: string): Map<string, string> {
  const ergebnis = new Map<string, string>();
  for (const [, name, wert] of block.matchAll(/(--[a-z0-9-]+):\s*([^;]+);/g)) {
    if (FARBWERT.test(wert)) ergebnis.set(name, wert.trim());
  }
  return ergebnis;
}

/** Relative Leuchtdichte nach WCAG 2.x für #rgb oder #rrggbb. */
function leuchtdichte(hex: string): number {
  let h = hex.replace("#", "");
  if (h.length === 3) h = [...h].map((z) => z + z).join("");
  expect(h, hex).toMatch(/^[0-9a-f]{6}$/i);
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(h.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function kontrast(a: string, b: string): number {
  const [hell, dunkel] = [leuchtdichte(a), leuchtdichte(b)].sort((x, y) => y - x);
  return (hell + 0.05) / (dunkel + 0.05);
}

/** Paare aus Vorder- und Hintergrund, die in beiden Darstellungen mindestens 4,5:1 erreichen müssen. */
const PAARE: [string, string, string][] = [
  ["Text auf Grund", "--text", "--grund"],
  ["Text auf Fläche", "--text", "--flaeche"],
  ["Text auf hellblauer Fläche", "--text", "--blau-hell"],
  ["leiser Text auf Grund", "--text-leise", "--grund"],
  ["leiser Text auf Fläche", "--text-leise", "--flaeche"],
  ["Links und Zweitknöpfe auf Grund", "--blau", "--grund"],
  ["Links und Zweitknöpfe auf Fläche", "--blau", "--flaeche"],
  ["Gewählter Ordner, Hover", "--blau-dunkel", "--blau-hell"],
  ["Hauptknopf", "--auf-farbe", "--blau"],
  ["Hauptknopf (Hover)", "--auf-farbe", "--blau-dunkel"],
  ["Gefahrknopf", "--auf-farbe", "--fehler"],
  ["Gefahrknopf (Hover)", "--auf-farbe", "--fehler-dunkel"],
  ["Warnung (Linkwarnung, Verbindung, Hinweise)", "--warnung", "--warnung-flaeche"],
  ["Fehler auf Fehlerfläche (Absenderwarnung, teilweise verschlüsselt)", "--fehler-dunkel", "--fehler-flaeche"],
  ["Fehler auf Fläche", "--fehler", "--flaeche"],
  ["Fehler auf Grund", "--fehler", "--grund"],
  ["Warnzeile OpenPGP auf Fläche", "--fehler-dunkel", "--flaeche"],
  ["Erfolg auf Fläche", "--erfolg", "--flaeche"],
  ["Tastaturfokus auf Grund", "--blau", "--grund"],
  ["Tastaturfokus auf Fläche", "--blau", "--flaeche"],
  ["Mailinhalt", "--mail-text", "--mail-grund"],
  ["Links im Mailinhalt", "--mail-link", "--mail-grund"],
];

describe("Design", () => {
  const css = readFileSync(join(SRC, "styles.css"), "utf8");
  const { block, rest } = zentralerBlock(css);
  const hell = farbvariablen(regelblock(block, /^:root,\s*::backdrop\s*\{/m));
  const dunkelSystem = farbvariablen(
    regelblock(block, /@media \(prefers-color-scheme: dark\)\s*\{\s*:root:not\(\[data-darstellung="hell"\]\),\s*:root:not\(\[data-darstellung="hell"\]\) ::backdrop\s*\{/),
  );
  const dunkelVonHand = farbvariablen(
    regelblock(block, /^:root\[data-darstellung="dunkel"\],\s*:root\[data-darstellung="dunkel"\] ::backdrop\s*\{/m),
  );

  it("Kontofarben (#50) sind in beiden Darstellungen gut sichtbar (mindestens 3:1)", () => {
    for (const [name, farben] of [
      ["hell", hell],
      ["dunkel", dunkelVonHand],
    ] as const) {
      for (let i = 1; i <= 6; i++) {
        const farbe = farben.get(`--konto-${i}`);
        expect(farbe, `${name}: --konto-${i}`).toBeDefined();
        for (const grund of ["--grund", "--flaeche", "--blau-hell"]) {
          expect(kontrast(farbe!, farben.get(grund)!), `${name}: --konto-${i} auf ${grund}`).toBeGreaterThanOrEqual(3);
        }
      }
    }
  });

  it("legt die Farben zentral fest", () => {
    expect(block).toMatch(/^\s*:root,\s*::backdrop\s*\{/m);
    for (const farbe of ["#2563eb", "#1d4ed8", "#eff6ff", "#dbeafe", "#ffffff", "#1f2937", "#6b7280", "#e5e7eb"]) {
      expect(block.toLowerCase()).toContain(farbe);
    }
  });

  it("enthält im zentralen Bereich nur die Variablenblöcke für hell und dunkel", () => {
    const ohneKommentare = block.replace(/\/\*[\s\S]*?\*\//g, "");
    const selektoren = [...ohneKommentare.matchAll(/([^{};]+)\{/g)].map((t) => t[1].trim().replace(/\s+/g, " "));
    expect(selektoren).toEqual([
      ":root, ::backdrop",
      "@media (prefers-color-scheme: dark)",
      ':root:not([data-darstellung="hell"]), :root:not([data-darstellung="hell"]) ::backdrop',
      ':root[data-darstellung="dunkel"], :root[data-darstellung="dunkel"] ::backdrop',
    ]);
    // In den Blöcken stehen nur Variablen und color-scheme.
    for (const [, eigenschaft] of ohneKommentare.matchAll(/^\s*([a-z][a-z-]*)\s*:/gm)) {
      expect(eigenschaft).toBe("color-scheme");
    }
  });

  it("verwendet ausserhalb der zentralen Definition keine festen Farbwerte", () => {
    const funde = rest.split("\n").filter((zeile) => FARBWERT.test(zeile));
    expect(funde).toEqual([]);
    expect(rest).not.toMatch(/--[a-z0-9-]+\s*:/);
    for (const datei of dateien(SRC).filter((d) => /\.(ts|html)$/.test(d))) {
      const zeilen = readFileSync(datei, "utf8")
        .split("\n")
        .filter((zeile) => FARBWERT.test(zeile));
      expect(zeilen, datei).toEqual([]);
    }
  });

  it("hat für jede helle Farbvariable eine dunkle, in beiden dunklen Blöcken gleich", () => {
    expect(hell.size).toBeGreaterThan(15);
    expect([...dunkelSystem.keys()].sort()).toEqual([...hell.keys()].sort());
    expect(dunkelVonHand).toEqual(dunkelSystem);
    expect(block).toMatch(/:root,\s*::backdrop\s*\{\s*color-scheme: light;/);
    expect(regelblock(block, /^:root\[data-darstellung="dunkel"\]/m)).toContain("color-scheme: dark;");
  });

  it("lässt den Mailinhalt auch dunkel auf hellem Grund", () => {
    for (const name of ["--mail-grund", "--mail-text", "--mail-link"]) {
      expect(dunkelSystem.get(name), name).toBe(hell.get(name));
    }
    expect(rest).toMatch(/iframe\.mail-rahmen\s*\{[^}]*background: var\(--mail-grund\)/);
    expect(rest).toMatch(/iframe\.mail-rahmen\s*\{[^}]*border: 1px solid var\(--linie\)/);
  });

  it.each([
    ["hell", hell],
    ["dunkel", dunkelSystem],
  ])("erreicht in der Darstellung „%s“ mindestens 4,5:1 Kontrast", (_name, farben) => {
    for (const [paar, vorne, hinten] of PAARE) {
      const verhaeltnis = kontrast(farben.get(vorne)!, farben.get(hinten)!);
      expect(verhaeltnis, `${paar}: ${vorne} auf ${hinten}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("dunkelt Dialoge auch in der dunklen Darstellung ab", () => {
    expect(rest).toMatch(/dialog::backdrop\s*\{\s*background: var\(--abdunkeln\)/);
    expect(dunkelSystem.get("--abdunkeln")).not.toBe(hell.get("--abdunkeln"));
    expect(dunkelSystem.get("--schatten-gross")).not.toBe(hell.get("--schatten-gross"));
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

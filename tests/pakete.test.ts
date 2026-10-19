import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const HAUPT = join(__dirname, "..");
const lesen = (pfad: string) => readFileSync(join(HAUPT, pfad), "utf8");

describe("Installationspakete (#48)", () => {
  it("fragt nirgends nach Updates", () => {
    const konfiguration = JSON.parse(lesen("src-tauri/tauri.conf.json")) as {
      plugins?: Record<string, unknown>;
      bundle: { createUpdaterArtifacts?: unknown };
    };
    expect(konfiguration.plugins?.updater).toBeUndefined();
    expect(konfiguration.bundle.createUpdaterArtifacts ?? false).toBe(false);
    for (const datei of ["src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "package.json", "package-lock.json"]) {
      expect(lesen(datei), datei).not.toMatch(/plugin-updater/);
    }
  });

  it("baut Pakete für alle drei Systeme", () => {
    const pakete = lesen(".github/workflows/pakete.yml");
    expect(pakete).toMatch(/^\s+workflow_call:/m);
    expect(pakete).toContain("--bundles deb,rpm,appimage");
    expect(pakete).toContain("--bundles msi,nsis");
    expect(pakete).toContain("--target universal-apple-darwin --bundles dmg");
    expect(pakete).toContain("-- --locked");
    expect(pakete).toContain("npm ci");
    expect(pakete).toContain("SOURCE_DATE_EPOCH");
  });

  it("veröffentlicht erst, wenn alle Pakete gebaut sind, mit Prüfsummen und Herkunftsnachweis", () => {
    const release = lesen(".github/workflows/release.yml");
    expect(release).toContain("uses: ./.github/workflows/pakete.yml");
    expect(release).toMatch(/needs: \[vorbereiten, pakete\]/);
    expect(release).toContain("SHA256SUMS.txt");
    // Pakete aus Unterordnern der Artefakte (deb/, rpm/, msi/ …) werden flach zusammengelegt.
    expect(release).toContain("find . -mindepth 2 -type f -exec mv -n -t . {} +");
    expect(release).toContain("actions/attest-build-provenance@");
    // Tag-Logik bleibt: Commit auf main, Versionsabgleich, bestehende Tags nie verschieben.
    expect(release).toContain("git merge-base --is-ancestor HEAD origin/main");
    expect(release).toContain('echo "Tag $tag gibt es schon"');
    // Fremde Actions sind auf Commits festgelegt.
    for (const [, aktion, ref] of `${release}\n${lesen(".github/workflows/pakete.yml")}`.matchAll(
      /uses: ((?:actions\/(?:upload|download)-artifact|actions\/attest-build-provenance|dtolnay\/rust-toolchain))@(\S+)/g,
    )) {
      expect(ref, aktion).toMatch(/^[0-9a-f]{40}$/);
    }
  });
});

/** Grundoberfläche von SicherMail: Kontenliste und Hauptbereich. */
import { api, fehlertext, type Konto } from "./api";
import { bestaetigen } from "./dialoge";
import { el } from "./dom";
import { kontoDialogOeffnen } from "./konto/dialog";

interface Zustand {
  konten: Konto[];
  fehler: string | null;
}

/** Baut die Grundoberfläche auf und lädt die Konten. */
export function appStarten(wurzel: HTMLElement): void {
  const zustand: Zustand = { konten: [], fehler: null };

  const kontenListe = el("ul", { class: "konten", "aria-label": "Konten" });
  const hinzufuegen = el("button", { type: "button", class: "haupt breit" }, "Konto hinzufügen");
  const inhalt = el("main", { class: "inhalt" });

  const seitenleiste = el(
    "aside",
    { class: "seitenleiste" },
    el("h1", {}, "SicherMail"),
    kontenListe,
    hinzufuegen,
  );
  wurzel.replaceChildren(el("div", { class: "layout" }, seitenleiste, inhalt));

  const zeichnen = () => {
    kontenListe.replaceChildren(
      ...zustand.konten.map((konto) => {
        const bearbeiten = el("button", { type: "button", class: "klein" }, "Bearbeiten");
        const loeschen = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
        bearbeiten.addEventListener("click", () => kontoDialogOeffnen(konto, () => void laden()));
        loeschen.addEventListener("click", async () => {
          const ok = await bestaetigen({
            titel: "Konto löschen?",
            text: `Das Konto „${konto.name}“ (${konto.email}) wird aus SicherMail entfernt, das Passwort aus dem Schlüsselbund gelöscht.\nDie Mails auf dem Server bleiben unverändert.`,
            ja: "Konto löschen",
            warnung: true,
          });
          if (!ok) return;
          try {
            await api.kontoLoeschen(konto.id);
          } catch (fehler) {
            zustand.fehler = fehlertext(fehler);
          }
          await laden();
        });
        return el(
          "li",
          { class: "konto" },
          el("div", { class: "konto-name" }, konto.name),
          el("div", { class: "konto-adresse" }, konto.email),
          el("div", { class: "konto-knoepfe" }, bearbeiten, loeschen),
        );
      }),
    );

    const meldungen: Node[] = [];
    if (zustand.fehler) {
      meldungen.push(el("p", { class: "fehler", role: "alert" }, zustand.fehler));
    }
    if (zustand.konten.length === 0) {
      meldungen.push(
        el(
          "section",
          { class: "willkommen" },
          el("h2", {}, "Willkommen bei SicherMail"),
          el(
            "p",
            {},
            "Richte ein Konto mit deinem eigenen IMAP- und SMTP-Server ein. SicherMail verbindet sich nur verschlüsselt und prüft die Zertifikate.",
          ),
        ),
      );
    } else {
      meldungen.push(el("p", { class: "leise" }, "Wähle ein Konto aus."));
    }
    inhalt.replaceChildren(...meldungen);
  };

  const laden = async () => {
    try {
      zustand.konten = await api.kontenAuflisten();
    } catch (fehler) {
      zustand.fehler = fehlertext(fehler);
    }
    zeichnen();
  };

  hinzufuegen.addEventListener("click", () => kontoDialogOeffnen(null, () => void laden()));

  zeichnen();
  void laden();
}

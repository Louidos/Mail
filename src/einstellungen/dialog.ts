/** Dialog „Einstellungen“: lokaler Speicher und freigegebene Absender. */
import { api, fehlertext, type Einstellungen } from "../api";
import { bestaetigen, modalSchliessen, modalZeigen } from "../dialoge";
import { el } from "../dom";

/**
 * Öffnet die Einstellungen. `geaendert` wird aufgerufen, wenn sich der lokale
 * Speicher geändert hat (die Oberfläche lädt dann die Mails neu).
 */
export async function einstellungenOeffnen(geaendert: () => void): Promise<HTMLDialogElement> {
  const status = el("p", { class: "status", role: "status", "aria-live": "polite" });
  const inhalt = el("div", {});
  const schliessen = el("button", { type: "button", class: "haupt" }, "Schliessen");
  const dialog = el(
    "dialog",
    { class: "einstellungen-dialog" },
    el("h2", {}, "Einstellungen"),
    inhalt,
    status,
    el("div", { class: "knoepfe" }, schliessen),
  );
  schliessen.addEventListener("click", () => modalSchliessen(dialog));
  dialog.addEventListener("cancel", (ereignis) => {
    ereignis.preventDefault();
    modalSchliessen(dialog);
  });

  const statusSetzen = (text: string, art: "info" | "erfolg" | "fehler") => {
    status.textContent = text;
    status.dataset.art = art;
  };

  const zeichnen = (e: Einstellungen) => {
    const lokal = el("input", { type: "checkbox", name: "lokal_speichern", checked: e.lokal_speichern });
    lokal.addEventListener("change", async () => {
      const an = lokal.checked;
      if (!an) {
        const ok = await bestaetigen({
          titel: "Lokale Speicherung ausschalten?",
          text: "Alle lokal gespeicherten Mails werden gelöscht. Ohne Internet sind danach keine Mails lesbar.\nAuf dem Server bleibt alles erhalten.",
          ja: "Ausschalten und löschen",
          warnung: true,
        });
        if (!ok) {
          lokal.checked = true;
          return;
        }
      }
      try {
        await api.lokalSpeichernSetzen(an);
        statusSetzen(an ? "Mails werden wieder verschlüsselt lokal gespeichert." : "Lokale Speicherung ist aus.", "erfolg");
        geaendert();
        await laden();
      } catch (fehler) {
        lokal.checked = !an;
        statusSetzen(fehlertext(fehler), "fehler");
      }
    });

    const leeren = el("button", { type: "button", class: "gefahr" }, "Lokalen Speicher leeren");
    leeren.addEventListener("click", async () => {
      const ok = await bestaetigen({
        titel: "Lokalen Speicher leeren?",
        text: "Alle zwischengespeicherten Mails werden von diesem Gerät gelöscht. Auf dem Server bleibt alles erhalten und wird bei Bedarf neu geladen.",
        ja: "Speicher leeren",
        warnung: true,
      });
      if (!ok) return;
      try {
        await api.lokalenSpeicherLeeren();
        statusSetzen("Der lokale Speicher wurde geleert.", "erfolg");
        geaendert();
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      }
    });

    const absender = el("ul", { class: "absender-liste" });
    for (const adresse of e.externe_inhalte_erlaubt) {
      const entfernen = el("button", { type: "button", class: "klein" }, "Entfernen");
      entfernen.addEventListener("click", async () => {
        try {
          await api.absenderExternEntfernen(adresse);
          await laden();
        } catch (fehler) {
          statusSetzen(fehlertext(fehler), "fehler");
        }
      });
      absender.append(el("li", {}, el("span", {}, adresse), entfernen));
    }

    inhalt.replaceChildren(
      el(
        "section",
        { class: "einstellung" },
        el("h3", {}, "Lokaler Speicher"),
        el("label", { class: "umschalter gross" }, lokal, "Mails lokal speichern (verschlüsselt)"),
        el(
          "p",
          { class: "leise" },
          "Heruntergeladene Mails werden in einer verschlüsselten Datenbank auf diesem Gerät abgelegt und sind so auch ohne Internet lesbar. Der Schlüssel liegt nur im Schlüsselbund des Betriebssystems.",
        ),
        e.speicher_hinweis ? el("p", { class: "warnung" }, e.speicher_hinweis) : null,
        leeren,
      ),
      el(
        "section",
        { class: "einstellung" },
        el("h3", {}, "Externe Inhalte immer erlaubt für"),
        e.externe_inhalte_erlaubt.length > 0 ? absender : el("p", { class: "leise" }, "Keine Absender freigegeben."),
      ),
    );
  };

  const laden = async () => {
    try {
      zeichnen(await api.einstellungenLaden());
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    }
  };

  modalZeigen(dialog);
  await laden();
  return dialog;
}

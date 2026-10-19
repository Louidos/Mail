/** Dialog „Einstellungen“: Darstellung, lokaler Speicher, Proxy für externe Bilder und freigegebene Absender. */
import { api, fehlertext, type Einstellungen } from "../api";
import { bestaetigen, modalSchliessen, modalZeigen } from "../dialoge";
import { el } from "../dom";
import { DARSTELLUNGEN, darstellungAnwenden, type Darstellung } from "./darstellung";

/** Warnhinweis zum Proxy für externe Bilder (#46). */
export const PROXY_HINWEIS =
  "Der Proxy löst Adressen selbst auf. SicherMail kann dann nicht mehr vollständig verhindern, dass Bilder aus deinem eigenen Netz geladen werden.";

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
    const darstellung = el(
      "select",
      { name: "darstellung", id: "einstellung-darstellung" },
      ...DARSTELLUNGEN.map(({ wert, text }) => el("option", { value: wert, selected: wert === e.darstellung }, text)),
    );
    let bisher: Darstellung = e.darstellung;
    darstellung.addEventListener("change", async () => {
      const neu = darstellung.value as Darstellung;
      // Sofort sichtbar; scheitert das Speichern, wird zurückgestellt.
      darstellungAnwenden(neu);
      try {
        await api.darstellungSetzen(neu);
        bisher = neu;
        statusSetzen("Die Darstellung wurde gespeichert.", "erfolg");
      } catch (fehler) {
        darstellungAnwenden(bisher);
        darstellung.value = bisher;
        statusSetzen(fehlertext(fehler), "fehler");
      }
    });

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

    const proxyAn = el("input", { type: "checkbox", name: "proxy_externe_bilder", checked: e.proxy_externe_bilder });
    const proxyAdresse = el("input", {
      type: "text",
      name: "proxy_adresse",
      value: e.proxy_adresse,
      placeholder: "http://proxy.example.org:3128",
      autocomplete: "off",
      spellcheck: "false",
      "aria-label": "Proxy-Adresse",
      disabled: !e.proxy_externe_bilder,
    });
    const proxyUebernehmen = el(
      "button",
      { type: "button", class: "klein", disabled: !e.proxy_externe_bilder },
      "Proxy übernehmen",
    );
    const proxySpeichern = async () => {
      try {
        await api.proxySetzen(true, proxyAdresse.value.trim());
        statusSetzen("Externe Bilder werden nach deiner Freigabe über den Proxy geladen.", "erfolg");
      } catch (fehler) {
        statusSetzen(`${fehlertext(fehler)} Externe Bilder bleiben blockiert.`, "fehler");
      }
    };
    proxyAn.addEventListener("change", async () => {
      proxyAdresse.disabled = !proxyAn.checked;
      proxyUebernehmen.disabled = !proxyAn.checked;
      if (proxyAn.checked) {
        // Gespeichert wird erst eine gültige Adresse („Proxy übernehmen“).
        if (proxyAdresse.value.trim()) {
          await proxySpeichern();
        } else {
          statusSetzen("Bitte die Proxy-Adresse eintragen und übernehmen.", "info");
          proxyAdresse.focus();
        }
        return;
      }
      try {
        await api.proxySetzen(false, proxyAdresse.value.trim());
        statusSetzen("Externe Bilder werden wieder direkt geladen, nie über einen Proxy.", "erfolg");
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      }
    });
    proxyUebernehmen.addEventListener("click", () => void proxySpeichern());

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
        el("h3", {}, "Darstellung"),
        el(
          "div",
          { class: "feld" },
          el("label", { class: "beschriftung", for: "einstellung-darstellung" }, "Hell oder dunkel"),
          darstellung,
        ),
        el(
          "p",
          { class: "leise" },
          "„Automatisch“ folgt der Einstellung deines Systems. Der Inhalt von Mails bleibt immer auf hellem Grund, weil viele Mails nur dafür gestaltet sind.",
        ),
      ),
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
        el("h3", {}, "Externe Bilder"),
        el("label", { class: "umschalter gross" }, proxyAn, "Proxy für externe Bilder verwenden"),
        el(
          "p",
          { class: "leise" },
          "Nur nötig, wenn dein Rechner ausschliesslich über einen Proxy ins Internet kommt. Ohne diese Einstellung lädt SicherMail externe Bilder immer direkt und nie über einen Proxy – auch nicht über einen im System eingestellten. Erlaubt ist eine Adresse mit http:// oder https://, Server und Port, ohne Benutzername und Passwort.",
        ),
        el("div", { class: "proxy-zeile" }, proxyAdresse, proxyUebernehmen),
        el("p", { class: "warnung", role: "note" }, PROXY_HINWEIS),
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

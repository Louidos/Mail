/** Liste der lokal gespeicherten Entwürfe eines Kontos. */
import { api, fehlertext, type Entwurf, type Konto } from "../api";
import { bestaetigen, modalSchliessen, modalZeigen } from "../dialoge";
import { el } from "../dom";

export async function entwuerfeOeffnen(konto: Konto, oeffnen: (entwurf: Entwurf) => void): Promise<HTMLDialogElement> {
  const liste = el("ul", { class: "entwurf-liste" });
  const status = el("p", { class: "status", role: "status" });
  const schliessen = el("button", { type: "button", class: "haupt" }, "Schliessen");
  const dialog = el(
    "dialog",
    { class: "entwuerfe-dialog" },
    el("h2", {}, `Entwürfe – ${konto.name}`),
    el("p", { class: "leise" }, "Entwürfe liegen nur auf diesem Gerät (verschlüsselt, wenn „Mails lokal speichern“ an ist)."),
    liste,
    status,
    el("div", { class: "knoepfe" }, schliessen),
  );
  schliessen.addEventListener("click", () => modalSchliessen(dialog));

  const laden = async () => {
    try {
      const entwuerfe = await api.entwuerfeAuflisten(konto.id);
      if (entwuerfe.length === 0) {
        liste.replaceChildren(el("li", { class: "leise" }, "Keine Entwürfe."));
        return;
      }
      liste.replaceChildren(
        ...entwuerfe.map((entwurf) => {
          const auf = el("button", { type: "button", class: "klein" }, "Öffnen");
          auf.addEventListener("click", () => {
            modalSchliessen(dialog);
            oeffnen(entwurf);
          });
          const weg = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
          weg.addEventListener("click", async () => {
            const ok = await bestaetigen({ titel: "Entwurf löschen?", text: entwurf.betreff || "(kein Betreff)", ja: "Löschen", warnung: true });
            if (!ok || !entwurf.id) return;
            await api.entwurfLoeschen(entwurf.id);
            await laden();
          });
          return el(
            "li",
            {},
            el("span", { class: "entwurf-betreff" }, entwurf.betreff || "(kein Betreff)"),
            el("span", { class: "leise" }, entwurf.an ? `an ${entwurf.an}` : "ohne Empfänger"),
            el("span", { class: "knoepfe-inline" }, auf, weg),
          );
        }),
      );
    } catch (fehler) {
      status.textContent = fehlertext(fehler);
    }
  };
  modalZeigen(dialog);
  await laden();
  return dialog;
}

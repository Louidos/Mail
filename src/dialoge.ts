/** Einfache modale Dialoge (statt window.confirm, das nicht überall funktioniert). */
import { el } from "./dom";

/** Zeigt einen modalen Dialog an. */
export function modalZeigen(dialog: HTMLDialogElement): void {
  document.body.append(dialog);
  if (typeof dialog.showModal === "function") {
    dialog.showModal();
  } else {
    dialog.setAttribute("open", "");
  }
}

/** Schliesst und entfernt einen Dialog. */
export function modalSchliessen(dialog: HTMLDialogElement): void {
  if (typeof dialog.close === "function") {
    dialog.close();
  }
  dialog.remove();
}

export interface BestaetigenOptionen {
  titel: string;
  text: string;
  /** Beschriftung des Knopfs, der bestätigt. */
  ja: string;
  nein?: string;
  /** Hebt den Dialog als Warnung hervor. */
  warnung?: boolean;
}

/** Fragt nach einer Bestätigung. Ergebnis `true` nur bei Klick auf „ja“. */
export function bestaetigen(optionen: BestaetigenOptionen): Promise<boolean> {
  return new Promise((erledigt) => {
    const ja = el("button", { type: "button", class: optionen.warnung ? "gefahr" : "haupt" }, optionen.ja);
    const nein = el("button", { type: "button" }, optionen.nein ?? "Abbrechen");
    const dialog = el(
      "dialog",
      { class: optionen.warnung ? "bestaetigen warnung" : "bestaetigen" },
      el("h2", {}, optionen.titel),
      el("p", { class: "vorformatiert" }, optionen.text),
      el("div", { class: "knoepfe" }, nein, ja),
    );
    const ende = (ergebnis: boolean) => {
      modalSchliessen(dialog);
      erledigt(ergebnis);
    };
    ja.addEventListener("click", () => ende(true));
    nein.addEventListener("click", () => ende(false));
    dialog.addEventListener("cancel", (ereignis) => {
      ereignis.preventDefault();
      ende(false);
    });
    modalZeigen(dialog);
    nein.focus();
  });
}

export interface PassphraseOptionen {
  titel: string;
  text: string;
  /** Beschriftung des Knopfs, der bestätigt. */
  ja: string;
}

/**
 * Fragt nach einer Passphrase. Ergebnis `null` bei Abbruch. Die Eingabe wird danach aus dem
 * Feld gelöscht und nirgends gespeichert.
 */
export function passphraseAbfragen(optionen: PassphraseOptionen): Promise<string | null> {
  return new Promise((erledigt) => {
    const feld = el("input", {
      type: "password",
      name: "passphrase",
      autocomplete: "off",
      "aria-label": "Passphrase",
      placeholder: "Passphrase",
    });
    const ja = el("button", { type: "submit", class: "haupt" }, optionen.ja);
    const nein = el("button", { type: "button" }, "Abbrechen");
    const formular = el(
      "form",
      { method: "dialog" },
      el("h2", {}, optionen.titel),
      el("p", { class: "vorformatiert" }, optionen.text),
      feld,
      el(
        "p",
        { class: "leise" },
        "Die Passphrase wird nicht gespeichert, sondern nur bis zum Beenden von SicherMail im Arbeitsspeicher gehalten.",
      ),
      el("div", { class: "knoepfe" }, nein, ja),
    );
    const dialog = el("dialog", { class: "bestaetigen passphrase" }, formular);
    const ende = (ergebnis: string | null) => {
      feld.value = "";
      modalSchliessen(dialog);
      erledigt(ergebnis);
    };
    formular.addEventListener("submit", (ereignis) => {
      ereignis.preventDefault();
      if (feld.value) ende(feld.value);
    });
    nein.addEventListener("click", () => ende(null));
    dialog.addEventListener("cancel", (ereignis) => {
      ereignis.preventDefault();
      ende(null);
    });
    modalZeigen(dialog);
    feld.focus();
  });
}

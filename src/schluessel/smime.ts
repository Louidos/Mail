/**
 * Bereich „S/MIME-Zertifikate“ im Dialog „Schlüssel und Zertifikate“ (#56): eigene Zertifikate
 * (PKCS#12) pro Konto importieren, entsperren und löschen; Zertifikate anderer importieren und
 * löschen; Wurzelzertifikate nach Bestätigung des Fingerabdrucks übernehmen und wieder entfernen.
 *
 * Dateien wählt ausschliesslich das Backend; die Oberfläche sieht nie einen privaten Schlüssel.
 * Passwörter werden nur durchgereicht, nie gespeichert.
 */
import { api, fehlertext, type EigenesZertifikat, type Konto, type ZertifikatInfo } from "../api";
import { bestaetigen } from "../dialoge";
import { el } from "../dom";
import { datumLang } from "../postfach/format";

type StatusSetzen = (text: string, art: "info" | "erfolg" | "fehler") => void;

/** Angaben zu einem Zertifikat: Name, Adressen, Aussteller, Gültigkeit und Fingerabdruck. */
export function zertifikatAngaben(info: ZertifikatInfo): Node[] {
  return [
    el("div", { class: "schluessel-ids" }, info.adressen.length > 0 ? `${info.name} <${info.adressen.join(", ")}>` : info.name),
    el("div", { class: "leise" }, `Ausgestellt von: ${info.aussteller}`),
    el(
      "div",
      { class: info.abgelaufen ? "fehler klein-text" : "leise" },
      info.abgelaufen ? `Abgelaufen am ${datumLang(info.gueltig_bis)}` : `Gültig bis ${datumLang(info.gueltig_bis)}`,
    ),
    el("div", { class: "fingerabdruck" }, el("span", { class: "leise" }, "Fingerabdruck (SHA-256): "), el("code", {}, info.fingerabdruck)),
  ];
}

/** Baut den Bereich; `laden` holt die Zertifikate (neu) vom Backend. */
export function smimeBereich(konten: Konto[], statusSetzen: StatusSetzen): { element: HTMLElement; laden: () => Promise<void> } {
  const element = el("section", { class: "smime-bereich" });

  const loeschenFragen = async (info: ZertifikatInfo, art: "eigenes" | "fremdes" | "wurzel") => {
    const texte = {
      eigenes: {
        titel: "Eigenes Zertifikat löschen?",
        text: `Das Zertifikat ${info.fingerabdruck} wird samt privatem Schlüssel endgültig von diesem Gerät gelöscht.\nMails, die an dieses Zertifikat verschlüsselt sind, kannst du danach hier nicht mehr lesen – ausser du hast die .p12-Datei noch.`,
        ja: "Endgültig löschen",
      },
      fremdes: { titel: "Zertifikat löschen?", text: `Das Zertifikat von ${info.name} wird gelöscht.`, ja: "Löschen" },
      wurzel: {
        titel: "Wurzelzertifikat entfernen?",
        text: `Signaturen, die nur über „${info.name}“ bestätigt werden, gelten danach als „unbekannte Stelle“.`,
        ja: "Entfernen",
      },
    }[art];
    if (!(await bestaetigen({ ...texte, warnung: true }))) return;
    try {
      await api.smimeLoeschen(info.fingerabdruck);
      statusSetzen(art === "wurzel" ? "Das Wurzelzertifikat wurde entfernt." : "Das Zertifikat wurde gelöscht.", "erfolg");
      await laden();
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    }
  };

  const eigenesEintrag = (zertifikat: EigenesZertifikat): HTMLElement => {
    const loeschen = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
    loeschen.addEventListener("click", () => void loeschenFragen(zertifikat, "eigenes"));
    const teile: Node[] = [...zertifikatAngaben(zertifikat)];
    if (zertifikat.entsperrt) {
      teile.push(el("div", { class: "leise" }, "Mit Passphrase geschützt, in dieser Sitzung entsperrt."));
      teile.push(el("div", { class: "konto-knoepfe" }, loeschen));
    } else {
      const feld = el("input", {
        type: "password",
        name: "smime_entsperren",
        autocomplete: "off",
        "aria-label": "Passphrase des Zertifikats",
        placeholder: "Passphrase (Passwort der .p12-Datei)",
      });
      const entsperren = el("button", { type: "button", class: "klein" }, "Entsperren");
      entsperren.addEventListener("click", async () => {
        const passphrase = feld.value;
        feld.value = "";
        if (!passphrase) return;
        try {
          await api.smimeEntsperren(zertifikat.fingerabdruck, passphrase);
          statusSetzen("Das Zertifikat ist bis zum Beenden von SicherMail entsperrt.", "erfolg");
          await laden();
        } catch (fehler) {
          statusSetzen(fehlertext(fehler), "fehler");
        }
      });
      teile.push(el("div", { class: "leise" }, "Mit Passphrase geschützt."));
      teile.push(el("div", { class: "passphrase-zeile" }, feld, entsperren, loeschen));
    }
    return el("li", {}, el("div", {}, ...teile));
  };

  const kontoBereich = (konto: Konto, eigene: EigenesZertifikat[]): HTMLElement => {
    const passwort = el("input", {
      type: "password",
      name: "smime_passwort",
      autocomplete: "off",
      "aria-label": "Passwort der .p12-Datei",
      placeholder: "Passwort der .p12/.pfx-Datei",
    });
    const importieren = el("button", { type: "button", class: "klein" }, "Eigenes Zertifikat importieren …");
    importieren.addEventListener("click", async () => {
      if (!passwort.value) {
        statusSetzen("Bitte zuerst das Passwort der .p12/.pfx-Datei eingeben.", "fehler");
        return;
      }
      importieren.disabled = true;
      try {
        const info = await api.smimeEigenesImportieren(konto.id, passwort.value);
        if (info) {
          statusSetzen(`Dein Zertifikat für ${info.adressen.join(", ")} wurde importiert.`, "erfolg");
          await laden();
        }
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      } finally {
        passwort.value = "";
        importieren.disabled = false;
      }
    });
    return el(
      "section",
      { class: "einstellung eigenes-zertifikat" },
      el("h3", {}, `${konto.name} <${konto.email}>`),
      eigene.length > 0
        ? el("ul", { class: "schluessel-liste" }, ...eigene.map(eigenesEintrag))
        : el("p", { class: "leise" }, "Noch kein eigenes S/MIME-Zertifikat."),
      el(
        "p",
        { class: "leise" },
        "Importiere dein Zertifikat mit privatem Schlüssel (.p12/.pfx). Es muss für die Adresse dieses Kontos ausgestellt sein. Das Passwort der Datei schützt danach auch den gespeicherten Schlüssel.",
      ),
      el("div", { class: "passphrase-zeile" }, passwort, importieren),
    );
  };

  const liste = (zertifikate: ZertifikatInfo[], art: "fremdes" | "wurzel", leer: string): HTMLElement => {
    if (zertifikate.length === 0) return el("p", { class: "leise" }, leer);
    const ul = el("ul", { class: "schluessel-liste" });
    for (const info of zertifikate) {
      const knopf = el("button", { type: "button", class: "klein gefahr" }, art === "wurzel" ? "Entfernen" : "Löschen");
      knopf.addEventListener("click", () => void loeschenFragen(info, art));
      ul.append(el("li", {}, el("div", {}, ...zertifikatAngaben(info)), knopf));
    }
    return ul;
  };

  const zeichnen = (eigene: EigenesZertifikat[], fremde: ZertifikatInfo[], wurzeln: ZertifikatInfo[]) => {
    const bereiche: Node[] = konten.map((konto) => kontoBereich(konto, eigene.filter((e) => e.konto === konto.id)));
    const verwaist = eigene.filter((e) => !konten.some((k) => k.id === e.konto));
    if (verwaist.length > 0) {
      bereiche.push(
        el(
          "section",
          { class: "einstellung" },
          el("h3", {}, "Zertifikate gelöschter Konten"),
          el("ul", { class: "schluessel-liste" }, ...verwaist.map(eigenesEintrag)),
        ),
      );
    }

    const fremdeImportieren = el("button", { type: "button", class: "klein" }, "Zertifikat importieren …");
    fremdeImportieren.addEventListener("click", async () => {
      try {
        const neu = await api.smimeFremdeImportieren();
        if (neu.length > 0) {
          statusSetzen(`Importiert: ${neu.map((i) => `${i.name} (${i.adressen.join(", ")})`).join("; ")}`, "erfolg");
          await laden();
        }
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      }
    });

    const wurzelUebernehmen = el("button", { type: "button", class: "klein" }, "Wurzelzertifikat übernehmen …");
    wurzelUebernehmen.addEventListener("click", async () => {
      try {
        const info = await api.smimeWurzelLesen();
        if (!info) return;
        const ok = await bestaetigen({
          titel: "Wurzelzertifikat übernehmen?",
          text: `Name: ${info.name}\nFingerabdruck (SHA-256):\n${info.fingerabdruck}\nGültig bis ${datumLang(info.gueltig_bis)}\n\nVergleiche den Fingerabdruck mit einer vertrauenswürdigen Quelle. Danach gelten alle Signaturen von Zertifikaten, die diese Stelle ausgestellt hat, als vertrauenswürdig.`,
          ja: "Wurzel übernehmen",
          warnung: true,
        });
        if (!ok) return;
        await api.smimeWurzelBestaetigen(info.fingerabdruck);
        statusSetzen(`Wurzelzertifikat „${info.name}“ übernommen.`, "erfolg");
        await laden();
      } catch (fehler) {
        statusSetzen(fehlertext(fehler), "fehler");
      }
    });

    element.replaceChildren(
      el("h2", { class: "abschnitt" }, "S/MIME-Zertifikate"),
      el(
        "p",
        { class: "leise" },
        "S/MIME nutzt Zertifikate einer Zertifizierungsstelle. Vertraut wird den Stellen im Zertifikatsspeicher deines Systems und den Wurzeln, die du hier ausdrücklich übernimmst. Sperrlisten werden nicht online abgefragt.",
      ),
      el("h3", { class: "abschnitt" }, "Eigene Zertifikate"),
      ...bereiche,
      ...(konten.length === 0 ? [el("p", { class: "leise" }, "Richte zuerst ein Konto ein.")] : []),
      el(
        "section",
        { class: "einstellung" },
        el("h3", {}, "Zertifikate anderer"),
        el("p", { class: "leise" }, "Aus einer Datei (.cer, .crt, .pem, .p7c) oder mit „Zertifikat übernehmen“ aus einer gültig signierten Mail."),
        liste(fremde, "fremdes", "Noch keine Zertifikate anderer."),
        fremdeImportieren,
      ),
      el(
        "section",
        { class: "einstellung" },
        el("h3", {}, "Übernommene Wurzelzertifikate"),
        liste(wurzeln, "wurzel", "Keine – es gelten nur die Stellen im Zertifikatsspeicher des Systems."),
        wurzelUebernehmen,
      ),
    );
  };

  const laden = async () => {
    try {
      const uebersicht = await api.smimeUebersicht();
      zeichnen(uebersicht?.eigene ?? [], uebersicht?.fremde ?? [], uebersicht?.wurzeln ?? []);
    } catch (fehler) {
      statusSetzen(fehlertext(fehler), "fehler");
    }
  };

  return { element, laden };
}

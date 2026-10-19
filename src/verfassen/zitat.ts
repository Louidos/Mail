/** Antworten, allen antworten und weiterleiten: Entwürfe aus einer gelesenen Mail. */
import type { Adresse, AufbereiteteMail, Entwurf, Konto } from "../api";
import { datumLang } from "../postfach/format";

/** Leerer Entwurf für ein Konto. */
export function leererEntwurf(konto: string): Entwurf {
  return {
    id: null,
    konto,
    an: "",
    cc: "",
    bcc: "",
    betreff: "",
    text: "",
    html: false,
    referenzen: [],
    anhaenge: [],
    verschluesseln: false,
    signieren: false,
  };
}

/** War die Mail verschlüsselt? Dann sind Antwort und Weiterleitung es standardmässig auch. */
function warVerschluesselt(mail: AufbereiteteMail): boolean {
  return (mail.pgp?.verschluesselung ?? "keine") !== "keine";
}

/** Bei verschlüsselten Mails antwortet man mit demselben Verfahren (OpenPGP oder S/MIME). */
function verfahrenWie(mail: AufbereiteteMail): Pick<Entwurf, "verfahren"> {
  return warVerschluesselt(mail) && mail.pgp?.verfahren ? { verfahren: mail.pgp.verfahren } : {};
}

/** Adresse für ein Empfängerfeld; Namen mit Sonderzeichen in Anführungszeichen. */
export function adresseFormatieren(a: Adresse): string {
  if (!a.name) return a.adresse;
  const name = /[,;:<>@"()[\]\\.]/.test(a.name) ? `"${a.name.replace(/["\\]/g, "")}"` : a.name;
  return `${name} <${a.adresse}>`;
}

function adressenListe(adressen: Adresse[]): string {
  return adressen.map(adresseFormatieren).join(", ");
}

/** Betreff mit „Re:“ bzw. „Fwd:“, ohne doppelte Vorsilben (auch „AW:“ und „WG:“ werden erkannt). */
export function betreffMitVorsilbe(betreff: string, art: "Re" | "Fwd"): string {
  const sauber = betreff.trim();
  const muster = art === "Re" ? /^(re|aw|antw)\s*:/i : /^(fwd?|wg)\s*:/i;
  return muster.test(sauber) ? sauber : `${art}: ${sauber}`;
}

/** Zitat der Mail mit Einleitungszeile, jede Zeile mit „> “. */
export function zitieren(mail: AufbereiteteMail): string {
  const von = mail.von.length > 0 ? adressenListe(mail.von) : "unbekannt";
  const zeilen = mail.text.replace(/\r\n/g, "\n").replace(/\n+$/, "").split("\n");
  return `Am ${datumLang(mail.datum)} schrieb ${von}:\n${zeilen.map((z) => (z.startsWith(">") ? `>${z}` : `> ${z}`)).join("\n")}\n`;
}

/** Entwurf für „Antworten“ bzw. „Allen antworten“. */
export function antwortEntwurf(mail: AufbereiteteMail, konto: Konto, allen: boolean): Entwurf {
  const eigene = konto.email.toLowerCase();
  const gesehen = new Set<string>([eigene]);
  const eindeutig = (liste: Adresse[]) =>
    liste.filter((a) => {
      const schluessel = a.adresse.toLowerCase();
      if (gesehen.has(schluessel)) return false;
      gesehen.add(schluessel);
      return true;
    });
  const an = eindeutig(mail.antwort_an.length > 0 ? mail.antwort_an : mail.von);
  const weitereAn = allen ? eindeutig(mail.an) : [];
  const cc = allen ? eindeutig(mail.cc) : [];
  return {
    ...leererEntwurf(konto.id),
    an: adressenListe([...an, ...weitereAn]),
    cc: adressenListe(cc),
    betreff: betreffMitVorsilbe(mail.betreff, "Re"),
    text: `\n\n${zitieren(mail)}`,
    antwort_auf: mail.message_id,
    referenzen: [...mail.referenzen],
    verschluesseln: warVerschluesselt(mail),
    signieren: warVerschluesselt(mail),
    ...verfahrenWie(mail),
  };
}

/** Entwurf für „Weiterleiten“ (Anhänge übernimmt die Oberfläche über das Backend). */
export function weiterleitenEntwurf(mail: AufbereiteteMail, konto: Konto): Entwurf {
  const kopf = [
    "---------- Weitergeleitete Nachricht ----------",
    `Von: ${adressenListe(mail.von)}`,
    `Datum: ${datumLang(mail.datum)}`,
    `Betreff: ${mail.betreff}`,
    `An: ${adressenListe(mail.an)}`,
  ];
  if (mail.cc.length > 0) kopf.push(`Cc: ${adressenListe(mail.cc)}`);
  return {
    ...leererEntwurf(konto.id),
    betreff: betreffMitVorsilbe(mail.betreff, "Fwd"),
    text: `\n\n${kopf.join("\n")}\n\n${mail.text}`,
    verschluesseln: warVerschluesselt(mail),
    signieren: warVerschluesselt(mail),
    ...verfahrenWie(mail),
  };
}

/** Grobe Prüfung der Empfängerfelder vor dem Senden (das Backend prüft genau). */
export function empfaengerFehlen(entwurf: Entwurf): boolean {
  return [entwurf.an, entwurf.cc, entwurf.bcc].every((feld) => feld.trim() === "");
}

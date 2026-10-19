/**
 * Verbindung zum Rust-Backend. Das Frontend selbst hat keinen Netzwerk-
 * oder Dateizugriff; alles läuft über diese Befehle.
 */
import { invoke } from "@tauri-apps/api/core";

export type Verschluesselung = "tls" | "starttls";

export interface ServerEinstellung {
  host: string;
  port: number;
  verschluesselung: Verschluesselung;
  zertifikat?: string | null;
}

export interface Konto {
  id: string;
  name: string;
  email: string;
  benutzername: string;
  imap: ServerEinstellung;
  smtp: ServerEinstellung;
  /** Gesendete Mails im Ordner „Gesendet“ ablegen. */
  gesendet_ablegen: boolean;
  /** Server-Kennung für die Absenderprüfung; leer heisst: IMAP-Server. */
  absender_kennung: string;
}

export interface KontoEingabe {
  id?: string | null;
  name: string;
  email: string;
  benutzername: string;
  /** Leer beim Bearbeiten heisst: bisheriges Passwort behalten. */
  passwort?: string | null;
  imap: ServerEinstellung;
  smtp: ServerEinstellung;
  gesendet_ablegen: boolean;
  absender_kennung: string;
}

export interface Fehlermeldung {
  art: string;
  meldung: string;
}

export interface Pruefergebnis {
  imap: Fehlermeldung | null;
  smtp: Fehlermeldung | null;
}

export interface Vorschlag {
  benutzername: string;
  imap: ServerEinstellung;
  smtp: ServerEinstellung;
}

export type Sonderordner = "posteingang" | "gesendet" | "entwuerfe" | "papierkorb" | "spam" | "archiv";

export interface Ordner {
  name: string;
  anzeigename: string;
  trenner: string | null;
  sonder: Sonderordner | null;
  waehlbar: boolean;
}

export interface MailKopf {
  uid: number;
  betreff: string;
  absender: string;
  absender_adresse: string;
  /** Unix-Zeit in Sekunden. */
  datum: number | null;
  gelesen: boolean;
  markiert: boolean;
  groesse: number;
}

export interface MailSeite {
  mails: MailKopf[];
  auf_server: number | null;
  offline: string | null;
  weitere: boolean;
}

/** Ereignisse des Backends (Ereignisname `mail-ereignis`). */
export type MailEreignis =
  | { art: "mails_geaendert"; konto: string; ordner: string }
  | { art: "verbunden"; konto: string; idle: boolean }
  | { art: "getrennt"; konto: string; meldung: string; sekunden: number };

export const MAIL_EREIGNIS = "mail-ereignis";

export interface Adresse {
  name: string | null;
  adresse: string;
}

export interface Anhang {
  index: number;
  name: string;
  typ: string;
  groesse: number;
}

/** Stufe der Absenderprüfung (#45). */
export type AbsenderStufe = "bestaetigt" | "nicht_geprueft" | "warnung";

/** Ergebnis einer Prüfmethode (DKIM, SPF oder DMARC). */
export interface PruefMethode {
  ergebnis: string;
  domain: string | null;
  /** Passt die Domain zur Absenderdomain? */
  passend: boolean;
}

/** Ergebnis der Absenderprüfung des eigenen Servers. */
export interface Absenderpruefung {
  stufe: AbsenderStufe;
  /** Server-Kennung (authserv-id), der vertraut wird. */
  kennung: string;
  /** Wurde ein Prüfergebnis mit dieser Kennung gefunden? */
  gefunden: boolean;
  absender_domain: string | null;
  dkim: PruefMethode[];
  spf: PruefMethode | null;
  dmarc: PruefMethode | null;
}

/** Zustand der Verschlüsselung (OpenPGP, #44). */
export type PgpVerschluesselung = "keine" | "entschluesselt" | "nicht_entschluesselt" | "teilweise";

/** Ergebnis der Signaturprüfung (OpenPGP). */
export type PgpSignatur = "keine" | "unbekannt" | "ungueltig" | "andere_adresse" | "gueltig";

/** Ergebnis von Entschlüsselung und Signaturprüfung. */
export interface PgpStatus {
  verschluesselung: PgpVerschluesselung;
  signatur: PgpSignatur;
  /** Nur ein Teil der Mail ist signiert. */
  signatur_teilweise: boolean;
  /** User-ID des Signaturschlüssels. */
  signierer: string | null;
  /** Fingerabdruck bzw. Schlüssel-ID des Signaturschlüssels. */
  signierer_schluessel: string | null;
  meldung: string | null;
  /** Fingerabdruck des eigenen Schlüssels, dessen Passphrase fehlt. */
  passphrase_fuer: string | null;
}

/** Ein OpenPGP-Schlüssel (ohne geheime Teile). */
export interface SchluesselInfo {
  fingerabdruck: string;
  benutzer_ids: string[];
  adressen: string[];
  /** Unix-Zeit in Sekunden. */
  erstellt: number;
  kann_verschluesseln: boolean;
}

export interface EigenerSchluessel extends SchluesselInfo {
  konto: string;
  geschuetzt: boolean;
  /** Passphrase in dieser Sitzung eingegeben (oder nicht nötig). */
  entsperrt: boolean;
}

export interface SchluesselUebersicht {
  eigene: EigenerSchluessel[];
  fremde: SchluesselInfo[];
}

/** Vom Backend bereinigte Mail. */
export interface AufbereiteteMail {
  betreff: string;
  von: Adresse[];
  an: Adresse[];
  cc: Adresse[];
  antwort_an: Adresse[];
  datum: number | null;
  message_id: string | null;
  referenzen: string[];
  text: string;
  /** Bereinigtes HTML (ohne Skripte, ohne externe Inhalte, sofern nicht erlaubt). */
  html: string | null;
  extern_blockiert: number;
  extern_geladen: boolean;
  anhaenge: Anhang[];
  absender_pruefung: Absenderpruefung;
  pgp: PgpStatus;
}

export interface Einstellungen {
  lokal_speichern: boolean;
  externe_inhalte_erlaubt: string[];
  /** Externe Bilder über den eingetragenen Proxy laden (standardmässig aus). */
  proxy_externe_bilder: boolean;
  /** Proxy-Adresse, z. B. `http://proxy.example.org:3128`. */
  proxy_adresse: string;
  /** Gesetzt, wenn der verschlüsselte Speicher nicht verfügbar ist. */
  speicher_hinweis: string | null;
}

/** Ein gewählter Anhang (das Frontend kennt nur das Token, nie den Dateipfad). */
export interface AnhangAuswahl {
  token: string;
  name: string;
  typ: string;
  groesse: number;
}

export interface Entwurf {
  id?: string | null;
  konto: string;
  an: string;
  cc: string;
  bcc: string;
  betreff: string;
  text: string;
  html: boolean;
  antwort_auf?: string | null;
  referenzen: string[];
  anhaenge: AnhangAuswahl[];
  /** Mit OpenPGP verschlüsseln; fehlt ein Empfängerschlüssel, wird nicht gesendet. */
  verschluesseln?: boolean;
  /** Mit OpenPGP signieren. */
  signieren?: boolean;
}

/** Ergebnis der Prüfung vor dem Senden mit OpenPGP. */
export interface VersandPruefung {
  fehlende_empfaenger: string[];
  eigener_fehlt: boolean;
  /** Fingerabdruck des eigenen Schlüssels, dessen Passphrase zum Signieren fehlt. */
  passphrase_fuer: string | null;
}

export interface SendeErgebnis {
  abgelegt: boolean;
  hinweis: string | null;
}

/** Macht aus einem Fehler des Backends einen lesbaren Text. */
export function fehlertext(fehler: unknown): string {
  if (typeof fehler === "string") return fehler;
  if (fehler && typeof fehler === "object" && "meldung" in fehler) {
    return String((fehler as Fehlermeldung).meldung);
  }
  return "Unbekannter Fehler.";
}

export const api = {
  kontenAuflisten: () => invoke<Konto[]>("konten_auflisten"),
  kontoTesten: (eingabe: KontoEingabe) => invoke<Pruefergebnis>("konto_testen", { eingabe }),
  kontoSpeichern: (eingabe: KontoEingabe) => invoke<Konto>("konto_speichern", { eingabe }),
  kontoLoeschen: (id: string) => invoke<void>("konto_loeschen", { id }),
  autoconfigSuchen: (email: string) => invoke<Vorschlag | null>("autoconfig_suchen", { email }),
  ordnerAuflisten: (konto: string) => invoke<Ordner[]>("ordner_auflisten", { konto }),
  mailsAuflisten: (konto: string, ordner: string, start: number, anzahl: number) =>
    invoke<MailSeite>("mails_auflisten", { konto, ordner, start, anzahl }),
  mailsLokal: (konto: string, ordner: string, start: number, anzahl: number) =>
    invoke<MailSeite>("mails_lokal", { konto, ordner, start, anzahl }),
  gelesenSetzen: (konto: string, ordner: string, uid: number, gelesen: boolean) =>
    invoke<void>("mail_gelesen_setzen", { konto, ordner, uid, gelesen }),
  mailLoeschen: (konto: string, ordner: string, uid: number) => invoke<void>("mail_loeschen", { konto, ordner, uid }),
  mailVerschieben: (konto: string, ordner: string, uid: number, ziel: string) =>
    invoke<void>("mail_verschieben", { konto, ordner, uid, ziel }),
  mailAnzeigen: (konto: string, ordner: string, uid: number, externLaden: boolean) =>
    invoke<AufbereiteteMail>("mail_anzeigen", { konto, ordner, uid, externLaden }),
  absenderExternErlauben: (adresse: string) => invoke<void>("absender_extern_erlauben", { adresse }),
  anhangSpeichern: (konto: string, ordner: string, uid: number, index: number) =>
    invoke<string | null>("anhang_speichern", { konto, ordner, uid, index }),
  linkOeffnen: (url: string) => invoke<void>("link_oeffnen", { url }),
  einstellungenLaden: () => invoke<Einstellungen>("einstellungen_laden"),
  lokalSpeichernSetzen: (an: boolean) => invoke<void>("lokal_speichern_setzen", { an }),
  lokalenSpeicherLeeren: () => invoke<void>("lokalen_speicher_leeren"),
  absenderExternEntfernen: (adresse: string) => invoke<void>("absender_extern_entfernen", { adresse }),
  proxySetzen: (an: boolean, adresse: string) => invoke<void>("proxy_setzen", { an, adresse }),
  anhangWaehlen: () => invoke<AnhangAuswahl[]>("anhang_waehlen"),
  mailAnhaengeUebernehmen: (konto: string, ordner: string, uid: number) =>
    invoke<AnhangAuswahl[]>("mail_anhaenge_uebernehmen", { konto, ordner, uid }),
  mailSenden: (entwurf: Entwurf) => invoke<SendeErgebnis>("mail_senden", { entwurf }),
  entwurfSpeichern: (entwurf: Entwurf) => invoke<Entwurf>("entwurf_speichern", { entwurf }),
  entwuerfeAuflisten: (konto: string) => invoke<Entwurf[]>("entwuerfe_auflisten", { konto }),
  entwurfLoeschen: (id: string) => invoke<void>("entwurf_loeschen", { id }),
  pgpUebersicht: () => invoke<SchluesselUebersicht>("pgp_uebersicht"),
  pgpErzeugen: (konto: string, passphrase: string) => invoke<SchluesselInfo>("pgp_erzeugen", { konto, passphrase }),
  pgpEigenenImportieren: (konto: string, passphrase: string) =>
    invoke<SchluesselInfo | null>("pgp_eigenen_importieren", { konto, passphrase }),
  pgpOeffentlicheImportieren: () => invoke<SchluesselInfo[]>("pgp_oeffentliche_importieren"),
  pgpAusAnhangImportieren: (konto: string, ordner: string, uid: number, index: number) =>
    invoke<SchluesselInfo[]>("pgp_aus_anhang_importieren", { konto, ordner, uid, index }),
  pgpExportieren: (fingerabdruck: string) => invoke<string | null>("pgp_exportieren", { fingerabdruck }),
  pgpLoeschen: (fingerabdruck: string) => invoke<void>("pgp_loeschen", { fingerabdruck }),
  pgpEntsperren: (fingerabdruck: string, passphrase: string) =>
    invoke<void>("pgp_entsperren", { fingerabdruck, passphrase }),
  pgpVersandPruefen: (entwurf: Entwurf) => invoke<VersandPruefung>("pgp_versand_pruefen", { entwurf }),
};

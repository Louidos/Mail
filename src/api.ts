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
};

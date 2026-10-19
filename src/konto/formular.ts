/** Reine Hilfsfunktionen für den Einrichtungsdialog (ohne Backend-Aufrufe). */
import type { Konto, KontoEingabe, ServerEinstellung, Verschluesselung } from "../api";

export type ServerArt = "imap" | "smtp";

/** Üblicher Port je Serverart und Verschlüsselung. */
export function standardPort(art: ServerArt, verschluesselung: Verschluesselung): number {
  if (art === "imap") return verschluesselung === "tls" ? 993 : 143;
  return verschluesselung === "tls" ? 465 : 587;
}

/** Ist der Port einer der Standardports dieser Serverart? */
export function istStandardPort(art: ServerArt, port: number): boolean {
  return port === standardPort(art, "tls") || port === standardPort(art, "starttls");
}

function wert(form: HTMLFormElement, name: string): string {
  const feld = form.elements.namedItem(name);
  if (feld instanceof HTMLInputElement || feld instanceof HTMLSelectElement || feld instanceof HTMLTextAreaElement) {
    return feld.value;
  }
  return "";
}

function setzen(form: HTMLFormElement, name: string, inhalt: string): void {
  const feld = form.elements.namedItem(name);
  if (feld instanceof HTMLInputElement || feld instanceof HTMLSelectElement || feld instanceof HTMLTextAreaElement) {
    feld.value = inhalt;
  }
}

function serverLesen(form: HTMLFormElement, art: ServerArt): ServerEinstellung {
  const verschluesselung = wert(form, `${art}_verschluesselung`) === "starttls" ? "starttls" : "tls";
  const zertifikat = wert(form, `${art}_zertifikat`).trim();
  return {
    host: wert(form, `${art}_host`).trim(),
    port: Number.parseInt(wert(form, `${art}_port`), 10) || 0,
    verschluesselung,
    zertifikat: zertifikat === "" ? null : zertifikat,
  };
}

/** Liest die Eingaben des Dialogs aus. */
export function formularLesen(form: HTMLFormElement, id: string | null): KontoEingabe {
  const passwort = wert(form, "passwort");
  return {
    id,
    name: wert(form, "name").trim(),
    email: wert(form, "email").trim(),
    benutzername: wert(form, "benutzername").trim(),
    passwort: passwort === "" ? null : passwort,
    imap: serverLesen(form, "imap"),
    smtp: serverLesen(form, "smtp"),
  };
}

/** Trägt Servereinstellungen in den Dialog ein. */
export function serverFuellen(form: HTMLFormElement, art: ServerArt, server: ServerEinstellung): void {
  setzen(form, `${art}_host`, server.host);
  setzen(form, `${art}_port`, String(server.port));
  setzen(form, `${art}_verschluesselung`, server.verschluesselung);
  setzen(form, `${art}_zertifikat`, server.zertifikat ?? "");
}

/** Füllt den Dialog mit einem bestehenden Konto (ohne Passwort). */
export function formularFuellen(form: HTMLFormElement, konto: Konto): void {
  setzen(form, "name", konto.name);
  setzen(form, "email", konto.email);
  setzen(form, "benutzername", konto.benutzername);
  setzen(form, "passwort", "");
  serverFuellen(form, "imap", konto.imap);
  serverFuellen(form, "smtp", konto.smtp);
}

/**
 * Prüft die Eingaben vor dem Senden ans Backend. Gibt eine Meldung zurück
 * oder `null`, wenn alles passt. Das Backend prüft zusätzlich.
 */
export function eingabePruefen(eingabe: KontoEingabe, neu: boolean): string | null {
  if (!eingabe.name) return "Bitte einen Namen angeben.";
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(eingabe.email)) return "Bitte eine gültige E-Mail-Adresse angeben.";
  if (!eingabe.benutzername) return "Bitte einen Benutzernamen angeben.";
  if (neu && !eingabe.passwort) return "Bitte das Passwort angeben.";
  for (const [art, server] of [
    ["IMAP", eingabe.imap],
    ["SMTP", eingabe.smtp],
  ] as const) {
    if (!server.host || /[\s/:]/.test(server.host)) {
      return `Bitte einen gültigen ${art}-Server angeben (nur der Name, z. B. mail.example.org).`;
    }
    if (!Number.isInteger(server.port) || server.port < 1 || server.port > 65535) {
      return `Bitte einen gültigen ${art}-Port angeben.`;
    }
  }
  return null;
}

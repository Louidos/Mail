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
};

/** Grundoberfläche von SicherMail: Konten und Ordner, Mailliste, Leseansicht. */
import { listen } from "@tauri-apps/api/event";
import {
  api,
  fehlertext,
  MAIL_EREIGNIS,
  type AufbereiteteMail,
  type Entwurf,
  type Konto,
  type MailEintrag,
  type MailEreignis,
  type MailKopf,
  type Ordner,
  type SendeErgebnis,
  type SuchFeld,
  type Verfahren,
} from "./api";
import { bestaetigen } from "./dialoge";
import { el } from "./dom";
import { einstellungenOeffnen } from "./einstellungen/dialog";
import { schluesselOeffnen } from "./schluessel/dialog";
import { verfassenOeffnen } from "./verfassen/dialog";
import { entwuerfeOeffnen } from "./verfassen/entwuerfe";
import { antwortEntwurf, leererEntwurf, weiterleitenEntwurf } from "./verfassen/zitat";
import { kontoDialogOeffnen } from "./konto/dialog";
import { datumKurz, ordnerKurzname, ordnerTiefe } from "./postfach/format";
import { leseansicht, type Inhalt } from "./postfach/lesen";
import { linkPruefen, type LinkKlick } from "./postfach/sicher";
import { hervorheben, SUCH_SEITE, Suchleiste, suchwoerter, type SuchBereich } from "./postfach/suche";

/** So viele Mails werden pro Seite geladen. */
export const SEITENGROESSE = 50;

/** Platzhalter, bis die Ordnerliste vom Server da ist. */
const POSTEINGANG: Ordner = {
  name: "INBOX",
  anzeigename: "Posteingang",
  trenner: null,
  sonder: "posteingang",
  waehlbar: true,
};

interface Auswahl {
  konto: string;
  ordner: string;
}

/** Dieselbe Mail (Konto, Ordner und UID)? */
export function gleicheMail(a: MailEintrag | null | undefined, b: MailEintrag | null | undefined): boolean {
  return !!a && !!b && a.konto === b.konto && a.ordner === b.ordner && a.uid === b.uid;
}

/** Laufende Suche (#49). */
interface SuchZustand {
  begriff: string;
  feld: SuchFeld;
  bereich: SuchBereich;
  /** Treffer vom Server statt aus dem lokalen Speicher. */
  server: boolean;
  treffer: MailEintrag[];
  weitere: boolean;
  laedt: boolean;
  fehler: string | null;
}

interface Zustand {
  konten: Konto[];
  ordner: Map<string, Ordner[]>;
  ordnerFehler: Map<string, string>;
  /** Meldung pro Konto, wenn die Verbindung unterbrochen ist. */
  verbindung: Map<string, string>;
  auswahl: Auswahl | null;
  mails: MailEintrag[];
  weitere: boolean;
  offline: string | null;
  laedt: boolean;
  offen: MailEintrag | null;
  /** Aktive Suche; `null` zeigt die normale Mailliste. */
  suche: SuchZustand | null;
  /** Inhalt der geöffneten Mail. */
  inhalt: Inhalt | null;
  /** „Nur Text anzeigen“ */
  nurText: boolean;
  /** Kurze Rückmeldung in der Leseansicht. */
  hinweis: string | null;
  fehler: string | null;
}

export class App {
  private zustand: Zustand = {
    konten: [],
    ordner: new Map(),
    ordnerFehler: new Map(),
    verbindung: new Map(),
    auswahl: null,
    mails: [],
    weitere: false,
    offline: null,
    laedt: false,
    offen: null,
    suche: null,
    inhalt: null,
    nurText: false,
    hinweis: null,
    fehler: null,
  };

  /** Merkt sich, was in der Leseansicht gezeichnet ist, um unnötiges Neuzeichnen zu vermeiden. */
  private gezeichnet: unknown[] = [];

  private readonly kontenBereich = el("nav", { class: "konten", "aria-label": "Konten und Ordner" });
  private readonly listenInhalt = el("div", { class: "listen-inhalt" });
  private readonly suchleiste = new Suchleiste({
    suchen: () => void this.lokalSuchen("neu"),
    serverSuchen: () => void this.serverSuchen(),
    beenden: () => this.sucheBeenden(),
  });
  private readonly listenBereich = el(
    "section",
    { class: "mailliste", "aria-label": "Mails" },
    this.suchleiste.element,
    this.listenInhalt,
  );
  /** Zählt Suchanfragen, damit verspätete Antworten älterer Anfragen verworfen werden. */
  private suchNummer = 0;
  private readonly leseBereich = el("section", { class: "lesebereich", "aria-label": "Mail lesen" });
  private readonly statusLeiste = el("div", { class: "statusleiste", role: "status", "aria-live": "polite" });

  constructor(private readonly wurzel: HTMLElement) {}

  /** Baut die Oberfläche auf, lädt die Konten und hört auf Ereignisse des Backends. */
  async starten(): Promise<void> {
    const neueMail = el("button", { type: "button", class: "haupt breit" }, "Neue Mail");
    neueMail.addEventListener("click", () => this.verfassen(null));
    const hinzufuegen = el("button", { type: "button", class: "breit" }, "Konto hinzufügen");
    hinzufuegen.addEventListener("click", () => kontoDialogOeffnen(null, () => void this.kontenLaden()));
    const einstellungen = el("button", { type: "button", class: "breit" }, "Einstellungen");
    einstellungen.addEventListener("click", () => void einstellungenOeffnen(() => void this.mailsLaden(false)));
    const schluessel = el("button", { type: "button", class: "breit" }, "Schlüssel und Zertifikate");
    schluessel.addEventListener("click", () => void schluesselOeffnen(this.zustand.konten));
    const seitenleiste = el(
      "aside",
      { class: "seitenleiste" },
      el("h1", {}, "SicherMail"),
      neueMail,
      this.kontenBereich,
      hinzufuegen,
      schluessel,
      einstellungen,
    );
    this.wurzel.replaceChildren(
      el("div", { class: "layout" }, seitenleiste, this.listenBereich, this.leseBereich, this.statusLeiste),
    );
    this.zeichnen();

    // Ctrl+F (macOS: Cmd+F) springt ins Suchfeld – nicht, solange ein Dialog offen ist.
    document.addEventListener("keydown", (ereignis) => {
      if ((ereignis.ctrlKey || ereignis.metaKey) && !ereignis.altKey && ereignis.key.toLowerCase() === "f") {
        if (document.querySelector("dialog[open]") || this.suchleiste.element.hidden) return;
        ereignis.preventDefault();
        this.suchleiste.fokussieren();
      }
    });

    try {
      await listen<MailEreignis>(MAIL_EREIGNIS, (ereignis) => this.ereignis(ereignis.payload));
    } catch {
      // Ausserhalb von Tauri (z. B. in Tests) gibt es keine Ereignisse.
    }
    await this.kontenLaden();
  }

  /** Verarbeitet ein Ereignis des Backends. */
  ereignis(ereignis: MailEreignis): void {
    const { auswahl } = this.zustand;
    switch (ereignis.art) {
      case "mails_geaendert":
        if (auswahl && auswahl.konto === ereignis.konto && auswahl.ordner === ereignis.ordner) {
          void this.mailsLaden(false);
        }
        // Lokale Treffer aktuell halten (die Server-Suche wird nur auf Klick wiederholt).
        if (this.zustand.suche && !this.zustand.suche.server && auswahl?.konto === ereignis.konto) {
          void this.lokalSuchen("auffrischen");
        }
        break;
      case "verbunden":
        this.zustand.verbindung.delete(ereignis.konto);
        this.statusZeichnen();
        // Ordner nachladen, falls sie (z. B. offline) nicht geladen werden konnten.
        if (
          this.zustand.ordnerFehler.has(ereignis.konto) ||
          (this.zustand.ordner.get(ereignis.konto) ?? []).length <= 1
        ) {
          void this.ordnerLaden(ereignis.konto);
        }
        break;
      case "getrennt":
        this.zustand.verbindung.set(
          ereignis.konto,
          `${ereignis.meldung} Neuer Versuch in ${ereignis.sekunden} s.`,
        );
        this.statusZeichnen();
        break;
    }
  }

  private async kontenLaden(): Promise<void> {
    try {
      this.zustand.konten = await api.kontenAuflisten();
      this.zustand.fehler = null;
    } catch (fehler) {
      this.zustand.fehler = fehlertext(fehler);
    }
    const ids = new Set(this.zustand.konten.map((k) => k.id));
    if (this.zustand.auswahl && !ids.has(this.zustand.auswahl.konto)) {
      this.zustand.auswahl = null;
      this.zustand.mails = [];
      this.zustand.offen = null;
      this.sucheBeenden();
    }
    // Den Posteingang gibt es immer – er ist sofort wählbar, auch ohne Verbindung.
    for (const konto of this.zustand.konten) {
      if (!this.zustand.ordner.has(konto.id)) this.zustand.ordner.set(konto.id, [POSTEINGANG]);
    }
    this.zeichnen();
    const ordnerGeladen = Promise.all(this.zustand.konten.map((k) => this.ordnerLaden(k.id)));
    if (!this.zustand.auswahl && this.zustand.konten.length > 0) {
      await this.ordnerWaehlen({ konto: this.zustand.konten[0].id, ordner: "INBOX" });
    }
    await ordnerGeladen;
  }

  private async ordnerLaden(konto: string): Promise<void> {
    try {
      this.zustand.ordner.set(konto, await api.ordnerAuflisten(konto));
      this.zustand.ordnerFehler.delete(konto);
    } catch (fehler) {
      this.zustand.ordnerFehler.set(konto, fehlertext(fehler));
    }
    this.kontenZeichnen();
  }

  private async ordnerWaehlen(auswahl: Auswahl): Promise<void> {
    // Ein anderer Ordner beendet die Suche und zeigt seine Mails.
    if (this.zustand.suche) this.sucheBeenden();
    this.zustand.auswahl = auswahl;
    this.zustand.mails = [];
    this.zustand.offen = null;
    this.zustand.inhalt = null;
    this.zustand.offline = null;
    this.zeichnen();
    await this.mailsLaden(false);
  }

  /** Lädt die erste Seite neu (`mehr = false`) oder hängt die nächste Seite an. */
  private async mailsLaden(mehr: boolean): Promise<void> {
    const auswahl = this.zustand.auswahl;
    if (!auswahl) return;
    const start = mehr ? this.zustand.mails.length : 0;
    const anzahl = mehr ? SEITENGROESSE : Math.max(SEITENGROESSE, this.zustand.mails.length);
    const eintraege = (mails: MailKopf[]): MailEintrag[] =>
      mails.map((m) => ({ ...m, konto: auswahl.konto, ordner: auswahl.ordner }));
    this.zustand.laedt = true;
    // Erste Seite sofort aus dem lokalen Speicher zeigen, dann mit dem Server abgleichen.
    if (!mehr && this.zustand.mails.length === 0) {
      try {
        const lokal = await api.mailsLokal(auswahl.konto, auswahl.ordner, 0, anzahl);
        if (this.zustand.auswahl === auswahl && lokal.mails.length > 0) {
          this.zustand.mails = eintraege(lokal.mails);
          this.zustand.weitere = lokal.weitere;
        }
      } catch {
        // Ohne lokalen Speicher geht es direkt mit dem Server weiter.
      }
    }
    this.listeZeichnen();
    try {
      const seite = await api.mailsAuflisten(auswahl.konto, auswahl.ordner, start, anzahl);
      if (this.zustand.auswahl !== auswahl) return;
      this.zustand.mails = mehr ? [...this.zustand.mails, ...eintraege(seite.mails)] : eintraege(seite.mails);
      this.zustand.weitere = seite.weitere;
      this.zustand.offline = seite.offline;
      this.zustand.fehler = null;
      // Die geöffnete Mail aktualisieren – nur, wenn sie in diesem Ordner liegt (Suchtreffer können
      // aus anderen Ordnern stammen).
      const offen = this.zustand.offen;
      if (offen && offen.konto === auswahl.konto && offen.ordner === auswahl.ordner) {
        this.zustand.offen = this.zustand.mails.find((m) => gleicheMail(m, offen)) ?? null;
      }
    } catch (fehler) {
      this.zustand.fehler = fehlertext(fehler);
    } finally {
      this.zustand.laedt = false;
      this.zeichnen();
    }
  }

  /** Lokale Suche (#49): neu, nächste Seite („mehr“) oder still aktualisieren („auffrischen“). */
  private async lokalSuchen(art: "neu" | "mehr" | "auffrischen"): Promise<void> {
    const auswahl = this.zustand.auswahl;
    const begriff = this.suchleiste.begriff;
    if (!auswahl || suchwoerter(begriff).length === 0) {
      if (art === "neu") this.sucheBeenden(false);
      return;
    }
    const vorher = this.zustand.suche;
    const feld = this.suchleiste.suchFeld;
    const bereich = this.suchleiste.bereich;
    const bisher = vorher && !vorher.server ? vorher.treffer : [];
    const start = art === "mehr" ? bisher.length : 0;
    const anzahl = art === "auffrischen" ? Math.max(SUCH_SEITE, bisher.length) : SUCH_SEITE;
    const nr = ++this.suchNummer;
    this.zustand.suche = {
      begriff,
      feld,
      bereich,
      server: false,
      treffer: art === "neu" ? [] : bisher,
      weitere: art === "neu" ? false : (vorher?.weitere ?? false),
      laedt: art !== "auffrischen",
      fehler: null,
    };
    this.suchleiste.aktivSetzen(true);
    this.listeZeichnen();
    let ergebnis: Partial<SuchZustand>;
    try {
      const seite = await api.mailsSuchen(
        auswahl.konto,
        bereich === "ordner" ? auswahl.ordner : null,
        begriff,
        feld,
        start,
        anzahl,
      );
      ergebnis = { treffer: art === "mehr" ? [...bisher, ...seite.mails] : seite.mails, weitere: seite.weitere };
    } catch (fehler) {
      ergebnis = { fehler: fehlertext(fehler) };
    }
    if (nr !== this.suchNummer || !this.zustand.suche) return;
    this.zustand.suche = { ...this.zustand.suche, ...ergebnis, laedt: false };
    this.listeZeichnen();
  }

  /** Suche auf dem eigenen Server – nur nach Klick auf „Auf dem Server suchen“. */
  private async serverSuchen(): Promise<void> {
    const auswahl = this.zustand.auswahl;
    const begriff = this.suchleiste.begriff;
    if (!auswahl || suchwoerter(begriff).length === 0) return;
    const feld = this.suchleiste.suchFeld;
    const bereich = this.suchleiste.bereich;
    const nr = ++this.suchNummer;
    this.zustand.suche = { begriff, feld, bereich, server: true, treffer: [], weitere: false, laedt: true, fehler: null };
    this.suchleiste.aktivSetzen(true);
    this.listeZeichnen();
    let ergebnis: Partial<SuchZustand>;
    try {
      const seite = await api.mailsServerSuchen(
        auswahl.konto,
        bereich === "ordner" ? auswahl.ordner : null,
        begriff,
        feld,
      );
      ergebnis = { treffer: seite.mails, weitere: seite.weitere };
    } catch (fehler) {
      ergebnis = { fehler: fehlertext(fehler) };
    }
    if (nr !== this.suchNummer || !this.zustand.suche) return;
    this.zustand.suche = { ...this.zustand.suche, ...ergebnis, laedt: false };
    this.listeZeichnen();
  }

  /** Beendet die Suche und zeigt wieder die Mailliste des Ordners. */
  private sucheBeenden(feldLeeren = true): void {
    this.suchNummer++;
    this.zustand.suche = null;
    if (feldLeeren) this.suchleiste.leeren();
    this.suchleiste.aktivSetzen(false);
    this.listeZeichnen();
  }

  private async mailOeffnen(mail: MailEintrag): Promise<void> {
    this.zustand.offen = mail;
    this.zustand.hinweis = null;
    const laden = this.inhaltLaden(false);
    if (!mail.gelesen) {
      await this.gelesenSetzen(mail, true);
    }
    await laden;
  }

  /** Lädt den bereinigten Inhalt der geöffneten Mail (optional mit externen Inhalten). */
  private async inhaltLaden(externLaden: boolean): Promise<void> {
    const { offen } = this.zustand;
    if (!offen) return;
    this.zustand.inhalt = { art: "laedt" };
    this.zeichnen();
    let inhalt: Inhalt;
    try {
      inhalt = { art: "fertig", mail: await api.mailAnzeigen(offen.konto, offen.ordner, offen.uid, externLaden) };
    } catch (fehler) {
      inhalt = { art: "fehler", meldung: fehlertext(fehler) };
    }
    // Inzwischen eine andere Mail geöffnet? Dann verwerfen.
    if (!gleicheMail(this.zustand.offen, offen)) return;
    this.zustand.inhalt = inhalt;
    this.zeichnen();
  }

  /** Klick auf einen Link in einer Mail: prüfen, bei Täuschung warnen, dann im Browser öffnen. */
  async linkKlick(klick: LinkKlick): Promise<void> {
    const warnung = linkPruefen(klick.text, klick.href);
    if (warnung) {
      const ok = await bestaetigen({
        titel: "Achtung: verdächtiger Link",
        text: `${warnung}\n\nEchtes Ziel:\n${klick.href}`,
        ja: "Trotzdem öffnen",
        nein: "Nicht öffnen",
        warnung: true,
      });
      if (!ok) return;
    }
    try {
      await api.linkOeffnen(klick.href);
    } catch (fehler) {
      this.zustand.hinweis = fehlertext(fehler);
      this.zeichnen();
    }
  }

  private async anhangSpeichern(index: number): Promise<void> {
    const { offen } = this.zustand;
    if (!offen) return;
    try {
      const name = await api.anhangSpeichern(offen.konto, offen.ordner, offen.uid, index);
      this.zustand.hinweis = name ? `Gespeichert: ${name}` : null;
    } catch (fehler) {
      this.zustand.hinweis = fehlertext(fehler);
    }
    this.zeichnen();
  }

  private async absenderErlauben(adresse: string): Promise<void> {
    try {
      await api.absenderExternErlauben(adresse);
    } catch (fehler) {
      this.zustand.hinweis = fehlertext(fehler);
    }
    await this.inhaltLaden(true);
  }

  /** Entsperrt den eigenen Schlüssel bzw. das S/MIME-Zertifikat und lädt die Mail neu (dann entschlüsselt). */
  private async pgpEntsperren(fingerabdruck: string, passphrase: string, verfahren: Verfahren): Promise<void> {
    try {
      if (verfahren === "smime") {
        await api.smimeEntsperren(fingerabdruck, passphrase);
      } else {
        await api.pgpEntsperren(fingerabdruck, passphrase);
      }
      this.zustand.hinweis = null;
      await this.inhaltLaden(false);
    } catch (fehler) {
      this.zustand.hinweis = fehlertext(fehler);
      this.zeichnen();
    }
  }

  /** S/MIME: übernimmt das Zertifikat des Absenders der geöffneten (gültig signierten) Mail. */
  private async zertifikatUebernehmen(): Promise<void> {
    const { offen } = this.zustand;
    if (!offen) return;
    try {
      const info = await api.smimeAusMailUebernehmen(offen.konto, offen.ordner, offen.uid);
      this.zustand.hinweis = `Zertifikat übernommen: ${info.name} (${info.adressen.join(", ")})`;
      await this.inhaltLaden(false);
      return;
    } catch (fehler) {
      this.zustand.hinweis = fehlertext(fehler);
    }
    this.zeichnen();
  }

  /** Importiert öffentliche Schlüssel aus einem Anhang der geöffneten Mail. */
  private async schluesselAusAnhang(index: number): Promise<void> {
    const { offen } = this.zustand;
    if (!offen) return;
    try {
      const infos = await api.pgpAusAnhangImportieren(offen.konto, offen.ordner, offen.uid, index);
      this.zustand.hinweis = `Importiert: ${infos.map((i) => i.benutzer_ids[0] ?? i.fingerabdruck).join(", ")}`;
      // Signaturen lassen sich jetzt eventuell prüfen.
      await this.inhaltLaden(false);
      return;
    } catch (fehler) {
      this.zustand.hinweis = fehlertext(fehler);
    }
    this.zeichnen();
  }

  private async gelesenSetzen(mail: MailEintrag, gelesen: boolean): Promise<void> {
    try {
      await api.gelesenSetzen(mail.konto, mail.ordner, mail.uid, gelesen);
      const geaendert = { ...mail, gelesen };
      const ersetzen = (liste: MailEintrag[]) => liste.map((m) => (gleicheMail(m, mail) ? { ...m, gelesen } : m));
      this.zustand.mails = ersetzen(this.zustand.mails);
      if (this.zustand.suche) this.zustand.suche = { ...this.zustand.suche, treffer: ersetzen(this.zustand.suche.treffer) };
      if (gleicheMail(this.zustand.offen, mail)) this.zustand.offen = geaendert;
    } catch (fehler) {
      this.zustand.fehler = fehlertext(fehler);
    }
    this.zeichnen();
  }

  private async mailEntfernen(mail: MailEintrag, aktion: () => Promise<void>): Promise<void> {
    try {
      await aktion();
      this.zustand.mails = this.zustand.mails.filter((m) => !gleicheMail(m, mail));
      if (this.zustand.suche) {
        this.zustand.suche = {
          ...this.zustand.suche,
          treffer: this.zustand.suche.treffer.filter((m) => !gleicheMail(m, mail)),
        };
      }
      if (gleicheMail(this.zustand.offen, mail)) {
        this.zustand.offen = null;
        this.zustand.inhalt = null;
      }
      this.zustand.fehler = null;
    } catch (fehler) {
      this.zustand.fehler = fehlertext(fehler);
    }
    this.zeichnen();
  }

  /**
   * Löscht eine Mail. Gibt es keinen Papierkorb oder liegt die Mail schon darin,
   * löscht der Server sie endgültig – dann wird vorher nachgefragt.
   */
  private async mailLoeschen(mail: MailEintrag): Promise<void> {
    const papierkorb = (this.zustand.ordner.get(mail.konto) ?? []).find((o) => o.sonder === "papierkorb");
    if (!papierkorb || papierkorb.name === mail.ordner) {
      const ok = await bestaetigen({
        titel: "Endgültig löschen?",
        text: `„${mail.betreff || "(kein Betreff)"}“ wird auf dem Server endgültig gelöscht und lässt sich nicht wiederherstellen.`,
        ja: "Endgültig löschen",
        warnung: true,
      });
      if (!ok) return;
    }
    await this.mailEntfernen(mail, () => api.mailLoeschen(mail.konto, mail.ordner, mail.uid));
  }

  /** Konto der aktuellen Auswahl (oder das erste). */
  private aktuellesKonto(): Konto | null {
    const id = this.zustand.auswahl?.konto;
    return this.zustand.konten.find((k) => k.id === id) ?? this.zustand.konten[0] ?? null;
  }

  /** Öffnet das Verfassen-Fenster mit einem Entwurf (oder leer). */
  verfassen(entwurf: Entwurf | null): void {
    const konto = this.aktuellesKonto();
    if (!konto) {
      this.meldungZeigen("Bitte zuerst ein Konto einrichten.");
      return;
    }
    verfassenOeffnen({
      konten: this.zustand.konten,
      entwurf: entwurf ?? leererEntwurf(konto.id),
      gesendet: (ergebnis) => this.gesendet(ergebnis),
    });
  }

  private antworten(mail: AufbereiteteMail, allen: boolean): void {
    const konto = this.aktuellesKonto();
    if (konto) this.verfassen(antwortEntwurf(mail, konto, allen));
  }

  private async weiterleiten(mail: AufbereiteteMail): Promise<void> {
    const { offen } = this.zustand;
    const konto = this.aktuellesKonto();
    if (!konto || !offen) return;
    const entwurf = weiterleitenEntwurf(mail, konto);
    if (mail.anhaenge.length > 0) {
      try {
        entwurf.anhaenge = await api.mailAnhaengeUebernehmen(offen.konto, offen.ordner, offen.uid);
      } catch (fehler) {
        this.meldungZeigen(`Anhänge konnten nicht übernommen werden: ${fehlertext(fehler)}`);
      }
    }
    this.verfassen(entwurf);
  }

  private gesendet(ergebnis: SendeErgebnis): void {
    this.meldungZeigen(ergebnis.hinweis ?? "Die Mail wurde gesendet.");
    // Der Ordner „Gesendet“ wurde eventuell gerade angelegt.
    for (const konto of this.zustand.konten) void this.ordnerLaden(konto.id);
    const ordner = this.zustand.auswahl ? this.zustand.ordner.get(this.zustand.auswahl.konto) : undefined;
    const aktuell = ordner?.find((o) => o.name === this.zustand.auswahl?.ordner);
    if (aktuell?.sonder === "gesendet") void this.mailsLaden(false);
  }

  private meldungTimer: ReturnType<typeof setTimeout> | null = null;
  private meldung: string | null = null;

  /** Kurze Rückmeldung unten in der Statusleiste. */
  private meldungZeigen(text: string): void {
    this.meldung = text;
    this.statusZeichnen();
    if (this.meldungTimer) clearTimeout(this.meldungTimer);
    this.meldungTimer = setTimeout(() => {
      this.meldung = null;
      this.statusZeichnen();
    }, 8000);
  }

  private zeichnen(): void {
    this.kontenZeichnen();
    this.listeZeichnen();
    this.leseansichtZeichnen();
    this.statusZeichnen();
  }

  private kontenZeichnen(): void {
    const { konten, ordner, ordnerFehler, auswahl } = this.zustand;
    this.kontenBereich.replaceChildren(
      ...konten.map((konto) => {
        const bearbeiten = el("button", { type: "button", class: "klein" }, "Bearbeiten");
        bearbeiten.addEventListener("click", () => kontoDialogOeffnen(konto, () => void this.kontenLaden()));
        const loeschen = el("button", { type: "button", class: "klein gefahr" }, "Löschen");
        loeschen.addEventListener("click", () => void this.kontoLoeschen(konto));
        const entwuerfe = el("button", { type: "button", class: "klein" }, "Entwürfe");
        entwuerfe.addEventListener("click", () => void entwuerfeOeffnen(konto, (e) => this.verfassen(e)));

        const liste = el("ul", { class: "ordner" });
        for (const o of ordner.get(konto.id) ?? []) {
          const gewaehlt = auswahl?.konto === konto.id && auswahl.ordner === o.name;
          const knopf = el(
            "button",
            {
              type: "button",
              class: gewaehlt ? "ordner-knopf gewaehlt" : "ordner-knopf",
              disabled: !o.waehlbar,
              "aria-current": gewaehlt ? "true" : undefined,
              style: `--tiefe: ${ordnerTiefe(o.name, o.trenner)}`,
            },
            o.sonder === "posteingang" ? o.anzeigename : ordnerKurzname(o.anzeigename, o.trenner),
          );
          knopf.addEventListener("click", () => void this.ordnerWaehlen({ konto: konto.id, ordner: o.name }));
          liste.append(el("li", {}, knopf));
        }
        const fehler = ordnerFehler.get(konto.id);
        return el(
          "section",
          { class: "konto" },
          el("div", { class: "konto-name" }, konto.name),
          el("div", { class: "konto-adresse" }, konto.email),
          fehler ? el("p", { class: "fehler klein-text" }, fehler) : null,
          liste,
          el("div", { class: "konto-knoepfe" }, entwuerfe, bearbeiten, loeschen),
        );
      }),
    );
  }

  /** Eine Zeile der Mailliste; bei einer Suche mit hervorgehobenen Begriffen und ggf. dem Ordner. */
  private mailZeile(mail: MailEintrag, suche: SuchZustand | null): HTMLLIElement {
    const { offen } = this.zustand;
    const klassen = ["mail"];
    if (!mail.gelesen) klassen.push("ungelesen");
    const istOffen = gleicheMail(offen, mail);
    if (istOffen) klassen.push("offen");
    const text = (wert: string, betroffen: boolean): Node[] =>
      suche && betroffen ? hervorheben(wert, suche.begriff) : [document.createTextNode(wert)];
    const absender = mail.absender || mail.absender_adresse || "(unbekannt)";
    const ordner = suche?.bereich === "konto"
      ? (this.zustand.ordner.get(mail.konto) ?? []).find((o) => o.name === mail.ordner)?.anzeigename ?? mail.ordner
      : null;
    const zeile = el(
      "button",
      { type: "button", class: klassen.join(" "), "aria-current": istOffen ? "true" : undefined },
      el("span", { class: "mail-absender" }, ...text(absender, suche?.feld !== "betreff" && suche?.feld !== "inhalt")),
      el("span", { class: "mail-datum" }, datumKurz(mail.datum)),
      el(
        "span",
        { class: "mail-betreff" },
        ...text(mail.betreff || "(kein Betreff)", suche?.feld !== "absender" && suche?.feld !== "inhalt"),
      ),
      ordner ? el("span", { class: "mail-ordner" }, `Ordner: ${ordner}`) : null,
    );
    zeile.addEventListener("click", () => void this.mailOeffnen(mail));
    return el("li", {}, zeile);
  }

  private listeZeichnen(): void {
    const { auswahl, mails, weitere, offline, laedt, fehler, konten, suche } = this.zustand;
    this.suchleiste.element.hidden = konten.length === 0 || !auswahl;
    const teile: Node[] = [];
    if (fehler) teile.push(el("p", { class: "fehler", role: "alert" }, fehler));
    if (konten.length === 0) {
      teile.push(
        el(
          "section",
          { class: "willkommen" },
          el("h2", {}, "Willkommen bei SicherMail"),
          el(
            "p",
            {},
            "Richte ein Konto mit deinem eigenen IMAP- und SMTP-Server ein. SicherMail verbindet sich nur verschlüsselt und prüft die Zertifikate.",
          ),
        ),
      );
      this.listenInhalt.replaceChildren(...teile);
      return;
    }
    if (!auswahl) {
      teile.push(el("p", { class: "leise" }, "Wähle einen Ordner aus."));
      this.listenInhalt.replaceChildren(...teile);
      return;
    }
    if (suche) {
      this.trefferZeichnen(suche, teile);
      return;
    }
    if (offline) {
      teile.push(el("p", { class: "hinweis-leiste" }, `Keine Verbindung zum Server – gespeicherte Mails werden angezeigt. ${offline}`));
    }
    const liste = el("ul", { class: "mails" });
    for (const mail of mails) liste.append(this.mailZeile(mail, null));
    teile.push(liste);
    if (laedt) {
      teile.push(el("p", { class: "leise" }, "Mails werden geladen …"));
    } else if (mails.length === 0) {
      teile.push(el("p", { class: "leise" }, "Keine Mails in diesem Ordner."));
    } else if (weitere) {
      const mehr = el("button", { type: "button", class: "breit" }, "Ältere Mails laden");
      mehr.addEventListener("click", () => void this.mailsLaden(true));
      teile.push(mehr);
    }
    this.listenInhalt.replaceChildren(...teile);
  }

  /** Trefferliste einer Suche (#49). */
  private trefferZeichnen(suche: SuchZustand, teile: Node[]): void {
    const anzahl = suche.treffer.length;
    let info: string;
    if (suche.laedt && anzahl === 0) {
      info = suche.server ? "Suche auf dem Server …" : "Suche läuft …";
    } else if (suche.fehler) {
      info = "";
    } else if (anzahl === 0) {
      info = suche.server ? "Auch auf dem Server keine Treffer." : "Keine Treffer auf diesem Gerät.";
    } else {
      const mehr = suche.weitere && !suche.server ? "+" : "";
      info = `${anzahl}${mehr} ${suche.server ? "Treffer auf dem Server" : "Treffer"}`;
    }
    teile.push(el("p", { class: "such-info", role: "status" }, info));
    if (suche.fehler) teile.push(el("p", { class: "fehler", role: "alert" }, suche.fehler));
    const liste = el("ul", { class: "mails" });
    for (const mail of suche.treffer) liste.append(this.mailZeile(mail, suche));
    teile.push(liste);
    if (suche.laedt && anzahl > 0) {
      teile.push(el("p", { class: "leise" }, "Weitere Treffer werden geladen …"));
    } else if (!suche.laedt && suche.weitere && !suche.server) {
      const mehr = el("button", { type: "button", class: "breit" }, "Mehr laden");
      mehr.addEventListener("click", () => void this.lokalSuchen("mehr"));
      teile.push(mehr);
    } else if (!suche.laedt && suche.weitere && suche.server) {
      teile.push(el("p", { class: "leise" }, "Es werden die neuesten 200 Treffer angezeigt. Grenze die Suche weiter ein."));
    }
    if (!suche.server && !suche.laedt) {
      teile.push(
        el(
          "p",
          { class: "leise" },
          "Gesucht wurde nur in den Mails, die auf diesem Gerät gespeichert sind. Ältere Mails findest du mit „Auf dem Server suchen“.",
        ),
      );
    }
    this.listenInhalt.replaceChildren(...teile);
  }

  private leseansichtZeichnen(): void {
    const { offen, inhalt, nurText, hinweis } = this.zustand;
    const ordner = offen ? this.zustand.ordner.get(offen.konto) : undefined;
    // Nur neu zeichnen, wenn sich etwas Sichtbares geändert hat (sonst würde das iframe neu geladen).
    const stand = [offen?.konto, offen?.ordner, offen?.uid, offen?.gelesen, inhalt, nurText, hinweis, ordner];
    if (stand.length === this.gezeichnet.length && stand.every((wert, i) => wert === this.gezeichnet[i])) {
      return;
    }
    this.gezeichnet = stand;
    if (!offen) {
      this.leseBereich.replaceChildren(el("p", { class: "leise" }, "Keine Mail ausgewählt."));
      return;
    }
    this.leseBereich.replaceChildren(
      leseansicht(offen, offen.ordner, ordner ?? [], inhalt ?? { art: "laedt" }, { nurText, hinweis }, {
        antworten: (mail, allen) => this.antworten(mail, allen),
        weiterleiten: (mail) => void this.weiterleiten(mail),
        gelesenUmschalten: () => void this.gelesenSetzen(offen, !offen.gelesen),
        loeschen: () => void this.mailLoeschen(offen),
        verschieben: (_mail, ziel) =>
          void this.mailEntfernen(offen, () => api.mailVerschieben(offen.konto, offen.ordner, offen.uid, ziel)),
        nurTextUmschalten: (wert) => {
          this.zustand.nurText = wert;
          this.zeichnen();
        },
        externLaden: () => void this.inhaltLaden(true),
        absenderErlauben: (adresse) => void this.absenderErlauben(adresse),
        anhangSpeichern: (index) => void this.anhangSpeichern(index),
        linkKlick: (klick) => void this.linkKlick(klick),
        pgpEntsperren: (fingerabdruck, passphrase, verfahren) =>
          void this.pgpEntsperren(fingerabdruck, passphrase, verfahren),
        zertifikatUebernehmen: () => void this.zertifikatUebernehmen(),
        schluesselImportieren: (index) => void this.schluesselAusAnhang(index),
      }),
    );
  }

  private statusZeichnen(): void {
    const meldungen = [...this.zustand.verbindung.entries()].map(([id, text]) => {
      const name = this.zustand.konten.find((k) => k.id === id)?.name ?? "Konto";
      return el("div", { class: "verbindung-getrennt" }, `${name}: ${text}`);
    });
    if (this.meldung) meldungen.push(el("div", { class: "meldung" }, this.meldung));
    this.statusLeiste.replaceChildren(...meldungen);
  }

  private async kontoLoeschen(konto: Konto): Promise<void> {
    const ok = await bestaetigen({
      titel: "Konto löschen?",
      text: `Das Konto „${konto.name}“ (${konto.email}) wird aus SicherMail entfernt, das Passwort aus dem Schlüsselbund gelöscht.\nDie Mails auf dem Server bleiben unverändert.`,
      ja: "Konto löschen",
      warnung: true,
    });
    if (!ok) return;
    try {
      await api.kontoLoeschen(konto.id);
    } catch (fehler) {
      this.zustand.fehler = fehlertext(fehler);
    }
    this.zustand.ordner.delete(konto.id);
    this.zustand.verbindung.delete(konto.id);
    await this.kontenLaden();
  }
}

/** Baut die Grundoberfläche auf und lädt die Konten. */
export function appStarten(wurzel: HTMLElement): App {
  const app = new App(wurzel);
  void app.starten();
  return app;
}

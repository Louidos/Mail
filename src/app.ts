/** Grundoberfläche von SicherMail: Konten und Ordner, Mailliste, Leseansicht. */
import { listen } from "@tauri-apps/api/event";
import {
  api,
  fehlertext,
  MAIL_EREIGNIS,
  type AufbereiteteMail,
  type Entwurf,
  type Konto,
  type MailEreignis,
  type MailKopf,
  type Ordner,
  type SendeErgebnis,
} from "./api";
import { bestaetigen } from "./dialoge";
import { el } from "./dom";
import { einstellungenOeffnen } from "./einstellungen/dialog";
import { verfassenOeffnen } from "./verfassen/dialog";
import { entwuerfeOeffnen } from "./verfassen/entwuerfe";
import { antwortEntwurf, leererEntwurf, weiterleitenEntwurf } from "./verfassen/zitat";
import { kontoDialogOeffnen } from "./konto/dialog";
import { datumKurz, ordnerKurzname, ordnerTiefe } from "./postfach/format";
import { leseansicht, type Inhalt } from "./postfach/lesen";
import { linkPruefen, type LinkKlick } from "./postfach/sicher";

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

interface Zustand {
  konten: Konto[];
  ordner: Map<string, Ordner[]>;
  ordnerFehler: Map<string, string>;
  /** Meldung pro Konto, wenn die Verbindung unterbrochen ist. */
  verbindung: Map<string, string>;
  auswahl: Auswahl | null;
  mails: MailKopf[];
  weitere: boolean;
  offline: string | null;
  laedt: boolean;
  offen: MailKopf | null;
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
    inhalt: null,
    nurText: false,
    hinweis: null,
    fehler: null,
  };

  /** Merkt sich, was in der Leseansicht gezeichnet ist, um unnötiges Neuzeichnen zu vermeiden. */
  private gezeichnet: unknown[] = [];

  private readonly kontenBereich = el("nav", { class: "konten", "aria-label": "Konten und Ordner" });
  private readonly listenBereich = el("section", { class: "mailliste", "aria-label": "Mails" });
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
    const seitenleiste = el(
      "aside",
      { class: "seitenleiste" },
      el("h1", {}, "SicherMail"),
      neueMail,
      this.kontenBereich,
      hinzufuegen,
      einstellungen,
    );
    this.wurzel.replaceChildren(
      el("div", { class: "layout" }, seitenleiste, this.listenBereich, this.leseBereich, this.statusLeiste),
    );
    this.zeichnen();

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
    this.zustand.laedt = true;
    // Erste Seite sofort aus dem lokalen Speicher zeigen, dann mit dem Server abgleichen.
    if (!mehr && this.zustand.mails.length === 0) {
      try {
        const lokal = await api.mailsLokal(auswahl.konto, auswahl.ordner, 0, anzahl);
        if (this.zustand.auswahl === auswahl && lokal.mails.length > 0) {
          this.zustand.mails = lokal.mails;
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
      this.zustand.mails = mehr ? [...this.zustand.mails, ...seite.mails] : seite.mails;
      this.zustand.weitere = seite.weitere;
      this.zustand.offline = seite.offline;
      this.zustand.fehler = null;
      if (this.zustand.offen) {
        const uid = this.zustand.offen.uid;
        this.zustand.offen = this.zustand.mails.find((m) => m.uid === uid) ?? null;
      }
    } catch (fehler) {
      this.zustand.fehler = fehlertext(fehler);
    } finally {
      this.zustand.laedt = false;
      this.zeichnen();
    }
  }

  private async mailOeffnen(mail: MailKopf): Promise<void> {
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
    const { auswahl, offen } = this.zustand;
    if (!auswahl || !offen) return;
    this.zustand.inhalt = { art: "laedt" };
    this.zeichnen();
    let inhalt: Inhalt;
    try {
      inhalt = { art: "fertig", mail: await api.mailAnzeigen(auswahl.konto, auswahl.ordner, offen.uid, externLaden) };
    } catch (fehler) {
      inhalt = { art: "fehler", meldung: fehlertext(fehler) };
    }
    // Inzwischen eine andere Mail geöffnet? Dann verwerfen.
    if (this.zustand.offen?.uid !== offen.uid || this.zustand.auswahl !== auswahl) return;
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
    const { auswahl, offen } = this.zustand;
    if (!auswahl || !offen) return;
    try {
      const name = await api.anhangSpeichern(auswahl.konto, auswahl.ordner, offen.uid, index);
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

  private async gelesenSetzen(mail: MailKopf, gelesen: boolean): Promise<void> {
    const auswahl = this.zustand.auswahl;
    if (!auswahl) return;
    try {
      await api.gelesenSetzen(auswahl.konto, auswahl.ordner, mail.uid, gelesen);
      const geaendert = { ...mail, gelesen };
      this.zustand.mails = this.zustand.mails.map((m) => (m.uid === mail.uid ? geaendert : m));
      if (this.zustand.offen?.uid === mail.uid) this.zustand.offen = geaendert;
    } catch (fehler) {
      this.zustand.fehler = fehlertext(fehler);
    }
    this.zeichnen();
  }

  private async mailEntfernen(mail: MailKopf, aktion: () => Promise<void>): Promise<void> {
    try {
      await aktion();
      this.zustand.mails = this.zustand.mails.filter((m) => m.uid !== mail.uid);
      if (this.zustand.offen?.uid === mail.uid) {
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
  private async mailLoeschen(mail: MailKopf): Promise<void> {
    const auswahl = this.zustand.auswahl;
    if (!auswahl) return;
    const papierkorb = (this.zustand.ordner.get(auswahl.konto) ?? []).find((o) => o.sonder === "papierkorb");
    if (!papierkorb || papierkorb.name === auswahl.ordner) {
      const ok = await bestaetigen({
        titel: "Endgültig löschen?",
        text: `„${mail.betreff || "(kein Betreff)"}“ wird auf dem Server endgültig gelöscht und lässt sich nicht wiederherstellen.`,
        ja: "Endgültig löschen",
        warnung: true,
      });
      if (!ok) return;
    }
    await this.mailEntfernen(mail, () => api.mailLoeschen(auswahl.konto, auswahl.ordner, mail.uid));
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
    const { auswahl, offen } = this.zustand;
    const konto = this.aktuellesKonto();
    if (!konto || !auswahl || !offen) return;
    const entwurf = weiterleitenEntwurf(mail, konto);
    if (mail.anhaenge.length > 0) {
      try {
        entwurf.anhaenge = await api.mailAnhaengeUebernehmen(auswahl.konto, auswahl.ordner, offen.uid);
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

  private listeZeichnen(): void {
    const { auswahl, mails, weitere, offline, laedt, offen, fehler, konten } = this.zustand;
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
      this.listenBereich.replaceChildren(...teile);
      return;
    }
    if (!auswahl) {
      teile.push(el("p", { class: "leise" }, "Wähle einen Ordner aus."));
      this.listenBereich.replaceChildren(...teile);
      return;
    }
    if (offline) {
      teile.push(el("p", { class: "hinweis-leiste" }, `Keine Verbindung zum Server – gespeicherte Mails werden angezeigt. ${offline}`));
    }
    const liste = el("ul", { class: "mails" });
    for (const mail of mails) {
      const klassen = ["mail"];
      if (!mail.gelesen) klassen.push("ungelesen");
      if (offen?.uid === mail.uid) klassen.push("offen");
      const zeile = el(
        "button",
        { type: "button", class: klassen.join(" "), "aria-current": offen?.uid === mail.uid ? "true" : undefined },
        el("span", { class: "mail-absender" }, mail.absender || mail.absender_adresse || "(unbekannt)"),
        el("span", { class: "mail-datum" }, datumKurz(mail.datum)),
        el("span", { class: "mail-betreff" }, mail.betreff || "(kein Betreff)"),
      );
      zeile.addEventListener("click", () => void this.mailOeffnen(mail));
      liste.append(el("li", {}, zeile));
    }
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
    this.listenBereich.replaceChildren(...teile);
  }

  private leseansichtZeichnen(): void {
    const { offen, auswahl, inhalt, nurText, hinweis } = this.zustand;
    const ordner = auswahl ? this.zustand.ordner.get(auswahl.konto) : undefined;
    // Nur neu zeichnen, wenn sich etwas Sichtbares geändert hat (sonst würde das iframe neu geladen).
    const stand = [offen?.uid, offen?.gelesen, auswahl, inhalt, nurText, hinweis, ordner];
    if (stand.length === this.gezeichnet.length && stand.every((wert, i) => wert === this.gezeichnet[i])) {
      return;
    }
    this.gezeichnet = stand;
    if (!offen || !auswahl) {
      this.leseBereich.replaceChildren(el("p", { class: "leise" }, "Keine Mail ausgewählt."));
      return;
    }
    this.leseBereich.replaceChildren(
      leseansicht(offen, auswahl.ordner, ordner ?? [], inhalt ?? { art: "laedt" }, { nurText, hinweis }, {
        antworten: (mail, allen) => this.antworten(mail, allen),
        weiterleiten: (mail) => void this.weiterleiten(mail),
        gelesenUmschalten: (mail) => void this.gelesenSetzen(mail, !mail.gelesen),
        loeschen: (mail) => void this.mailLoeschen(mail),
        verschieben: (mail, ziel) =>
          void this.mailEntfernen(mail, () => api.mailVerschieben(auswahl.konto, auswahl.ordner, mail.uid, ziel)),
        nurTextUmschalten: (wert) => {
          this.zustand.nurText = wert;
          this.zeichnen();
        },
        externLaden: () => void this.inhaltLaden(true),
        absenderErlauben: (adresse) => void this.absenderErlauben(adresse),
        anhangSpeichern: (index) => void this.anhangSpeichern(index),
        linkKlick: (klick) => void this.linkKlick(klick),
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

import { describe, expect, it } from "vitest";
import type { Konto, KontoEingabe } from "../src/api";
import {
  eingabePruefen,
  formularFuellen,
  formularLesen,
  istStandardPort,
  standardPort,
} from "../src/konto/formular";

function formular(): HTMLFormElement {
  const form = document.createElement("form");
  for (const name of ["name", "email", "benutzername", "passwort", "imap_host", "imap_port", "smtp_host", "smtp_port"]) {
    const feld = document.createElement("input");
    feld.name = name;
    form.append(feld);
  }
  for (const art of ["imap", "smtp"]) {
    const auswahl = document.createElement("select");
    auswahl.name = `${art}_verschluesselung`;
    for (const wert of ["tls", "starttls"]) {
      const option = document.createElement("option");
      option.value = wert;
      auswahl.append(option);
    }
    form.append(auswahl);
    const zertifikat = document.createElement("textarea");
    zertifikat.name = `${art}_zertifikat`;
    form.append(zertifikat);
  }
  const kennung = document.createElement("input");
  kennung.name = "absender_kennung";
  form.append(kennung);
  const ablegen = document.createElement("input");
  ablegen.type = "checkbox";
  ablegen.name = "gesendet_ablegen";
  form.append(ablegen);
  return form;
}

const konto: Konto = {
  id: "k1",
  name: "Anna Muster",
  email: "anna@example.org",
  benutzername: "anna",
  imap: { host: "imap.example.org", port: 993, verschluesselung: "tls" },
  smtp: { host: "smtp.example.org", port: 587, verschluesselung: "starttls", zertifikat: "PEM" },
  gesendet_ablegen: false,
  absender_kennung: "mx.example.org",
};

const gueltig: KontoEingabe = { ...konto, passwort: "geheim" };

describe("Standardports", () => {
  it("passen zu Serverart und Verschlüsselung", () => {
    expect(standardPort("imap", "tls")).toBe(993);
    expect(standardPort("imap", "starttls")).toBe(143);
    expect(standardPort("smtp", "tls")).toBe(465);
    expect(standardPort("smtp", "starttls")).toBe(587);
    expect(istStandardPort("smtp", 587)).toBe(true);
    expect(istStandardPort("smtp", 2525)).toBe(false);
  });
});

describe("Formular", () => {
  it("füllt ein Konto ein und liest es wieder aus, ohne Passwort", () => {
    const form = formular();
    formularFuellen(form, konto);
    const eingabe = formularLesen(form, konto.id);
    expect(eingabe).toEqual({ ...konto, passwort: null, imap: { ...konto.imap, zertifikat: null } });
  });

  it("liest ein eingegebenes Passwort", () => {
    const form = formular();
    formularFuellen(form, konto);
    (form.elements.namedItem("passwort") as HTMLInputElement).value = "neu";
    expect(formularLesen(form, null).passwort).toBe("neu");
  });
});

describe("eingabePruefen", () => {
  it("akzeptiert gültige Eingaben", () => {
    expect(eingabePruefen(gueltig, true)).toBeNull();
  });

  it("verlangt beim Anlegen ein Passwort, beim Bearbeiten nicht", () => {
    expect(eingabePruefen({ ...gueltig, passwort: null }, true)).toMatch(/Passwort/);
    expect(eingabePruefen({ ...gueltig, passwort: null }, false)).toBeNull();
  });

  it("lehnt ungültige Adressen, Server und Ports ab", () => {
    expect(eingabePruefen({ ...gueltig, email: "anna" }, true)).toMatch(/E-Mail/);
    expect(eingabePruefen({ ...gueltig, imap: { ...gueltig.imap, host: "imap.example.org:993" } }, true)).toMatch(
      /IMAP-Server/,
    );
    expect(eingabePruefen({ ...gueltig, smtp: { ...gueltig.smtp, port: 70000 } }, true)).toMatch(/SMTP-Port/);
    expect(eingabePruefen({ ...gueltig, absender_kennung: "mx.example.org; x" }, true)).toMatch(/Server-Kennung/);
    expect(eingabePruefen({ ...gueltig, absender_kennung: "" }, true)).toBeNull();
  });
});

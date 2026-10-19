import { describe, expect, it, vi } from "vitest";
import {
  linkAusNavigation,
  linkPruefen,
  linksAbfangen,
  linksVorbereiten,
  MAIL_CSP,
  mailDokument,
  mailRahmen,
  textMitLinks,
} from "../src/postfach/sicher";

describe("abgeschottete Anzeige", () => {
  it("erlaubt im iframe keine Skripte, Formulare, Popups oder Navigation", () => {
    const rahmen = mailRahmen("<p>Hallo</p>", "Test");
    const sandbox = rahmen.getAttribute("sandbox");
    expect(sandbox).toBe("allow-same-origin");
    for (const verboten of ["allow-scripts", "allow-forms", "allow-popups", "allow-top-navigation", "allow-modals"]) {
      expect(sandbox).not.toContain(verboten);
    }
    expect(rahmen.getAttribute("referrerpolicy")).toBe("no-referrer");
  });

  it("setzt eine eigene CSP, die nur eingebettete Bilder erlaubt", () => {
    expect(MAIL_CSP).toContain("default-src 'none'");
    expect(MAIL_CSP).toContain("img-src data:");
    expect(MAIL_CSP).not.toMatch(/script-src|https?:/);
    const dokument = mailDokument("<p>x</p>", "Titel");
    expect(dokument).toContain(`<meta http-equiv="Content-Security-Policy" content="${MAIL_CSP}">`);
    // Die CSP steht vor dem Inhalt der Mail.
    expect(dokument.indexOf("Content-Security-Policy")).toBeLessThan(dokument.indexOf("<p>x</p>"));
  });

  it("maskiert den Titel", () => {
    expect(mailDokument("", "</title><script>alert(1)</script>")).not.toContain("<script>");
  });
});

describe("linkPruefen", () => {
  it("warnt, wenn der Text eine andere Adresse vortäuscht", () => {
    expect(linkPruefen("https://www.meine-bank.ch/login", "https://meine-bank.ch.boese.example/login")).toMatch(
      /führt aber zu „meine-bank.ch.boese.example“/,
    );
    expect(linkPruefen("www.post.ch", "http://paket-post.example/")).toMatch(/Phishing/);
    expect(linkPruefen("anna@example.org", "mailto:boese@example.net")).toMatch(/boese@example.net/);
  });

  it("warnt nicht bei passenden Adressen oder normalem Text", () => {
    expect(linkPruefen("https://example.org/a", "https://www.example.org/b")).toBeNull();
    expect(linkPruefen("Hier klicken", "https://example.org/")).toBeNull();
    expect(linkPruefen("anna@example.org", "mailto:Anna@Example.org")).toBeNull();
    expect(linkPruefen("Newsletter abbestellen", "mailto:abmelden@example.org")).toBeNull();
  });

  it("warnt bei Punycode-Adressen", () => {
    expect(linkPruefen("Konto prüfen", "https://xn--pple-43d.com/")).toMatch(/Sonderzeichen/);
  });
});

describe("Links abfangen", () => {
  it("öffnet Links nie direkt und meldet Ziel und Text", () => {
    const bereich = document.createElement("div");
    bereich.append(textMitLinks("Siehe https://example.org/pfad. Danke"));
    const a = bereich.querySelector("a")!;
    expect(a.getAttribute("href")).toBe("https://example.org/pfad");
    const klick = vi.fn();
    linksAbfangen(bereich, klick);
    expect(a.getAttribute("title")).toBe("https://example.org/pfad");
    const ereignis = new MouseEvent("click", { bubbles: true, cancelable: true });
    a.dispatchEvent(ereignis);
    expect(ereignis.defaultPrevented).toBe(true);
    expect(klick).toHaveBeenCalledWith({ href: "https://example.org/pfad", text: "https://example.org/pfad" });
  });

  it("macht aus Text keine HTML-Elemente ausser Links", () => {
    const bereich = document.createElement("div");
    bereich.append(textMitLinks("<img src=x onerror=alert(1)> https://a.example"));
    expect(bereich.querySelector("img")).toBeNull();
    expect(bereich.querySelectorAll("a")).toHaveLength(1);
  });
});

describe("Links im iframe", () => {
  it("schreibt Links auf harmlose Sprungmarken um und merkt sich Ziel und Text", () => {
    const dokument = document.implementation.createHTMLDocument("Mail");
    dokument.body.innerHTML =
      '<a href="https://boese.example/" target="_blank">https://meine-bank.ch</a> <a>ohne Ziel</a> <a href="mailto:a@example.org">Mail</a>';
    const links = linksVorbereiten(dokument);
    expect(links).toEqual([
      { href: "https://boese.example/", text: "https://meine-bank.ch" },
      { href: "mailto:a@example.org", text: "Mail" },
    ]);
    const anker = dokument.querySelectorAll("a");
    expect(anker[0].getAttribute("href")).toBe("about:blank#sichermail-link-0");
    expect(anker[0].getAttribute("target")).toBeNull();
    expect(anker[0].getAttribute("title")).toBe("https://boese.example/");
    expect(anker[1].hasAttribute("href")).toBe(false);
    expect(anker[2].getAttribute("href")).toBe("about:blank#sichermail-link-1");
    // Kein echtes Ziel bleibt im iframe anklickbar.
    expect(dokument.body.innerHTML).not.toMatch(/href="(https?|mailto):/);
  });

  it("erkennt den geklickten Link an der Sprungmarke", () => {
    const links = [
      { href: "https://a.example/", text: "A" },
      { href: "https://b.example/", text: "B" },
    ];
    expect(linkAusNavigation("#sichermail-link-1", links)).toEqual(links[1]);
    expect(linkAusNavigation("#sichermail-link-7", links)).toBeNull();
    expect(linkAusNavigation("#anderes", links)).toBeNull();
    expect(linkAusNavigation("", links)).toBeNull();
  });
});

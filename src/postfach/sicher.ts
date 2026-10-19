/**
 * Sichere Darstellung von Mailinhalten.
 *
 * - HTML kommt bereits bereinigt aus dem Backend und wird zusätzlich in einem
 *   abgeschotteten iframe angezeigt: `sandbox` ohne `allow-scripts`, ohne Formulare,
 *   ohne Popups und ohne Navigation, mit eigener strenger Content-Security-Policy,
 *   die nur `data:`-Bilder zulässt (also keinerlei Netzwerkzugriff).
 * - Links werden nie im iframe geöffnet, sondern abgefangen und – nach Prüfung –
 *   im Standardbrowser geöffnet.
 */

/** CSP innerhalb der Mailanzeige: nichts laden ausser eingebetteten Bildern. */
export const MAIL_CSP =
  "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; form-action 'none'; base-uri 'none'";

/** Einzige Freigabe des iframes: Das Hauptfenster darf Klicks auf Links abfangen. */
export const MAIL_SANDBOX = "allow-same-origin";

function escape(text: string): string {
  return text.replace(/[&<>"']/g, (z) => `&#${z.charCodeAt(0)};`);
}

/** Vollständiges Dokument für das iframe (`srcdoc`). */
export function mailDokument(bereinigtesHtml: string, titel: string): string {
  return [
    "<!doctype html><html><head><meta charset=\"utf-8\">",
    `<meta http-equiv="Content-Security-Policy" content="${MAIL_CSP}">`,
    '<meta name="referrer" content="no-referrer">',
    `<title>${escape(titel)}</title>`,
    "<style>html,body{margin:0;padding:0;background:#fff;color:#111;font-family:system-ui,-apple-system,\"Segoe UI\",Roboto,sans-serif;font-size:15px;line-height:1.45;overflow-wrap:anywhere}",
    "body{padding:12px}img{max-width:100%;height:auto}table{max-width:100%}a{color:#0b57d0;cursor:pointer}pre{white-space:pre-wrap}</style>",
    "</head><body>",
    bereinigtesHtml,
    "</body></html>",
  ].join("");
}

/** Abgeschottetes iframe für eine HTML-Mail. */
export function mailRahmen(bereinigtesHtml: string, titel: string): HTMLIFrameElement {
  const rahmen = document.createElement("iframe");
  rahmen.className = "mail-rahmen";
  rahmen.setAttribute("sandbox", MAIL_SANDBOX);
  rahmen.setAttribute("referrerpolicy", "no-referrer");
  rahmen.setAttribute("csp", MAIL_CSP);
  rahmen.setAttribute("title", `Inhalt: ${titel}`);
  rahmen.srcdoc = mailDokument(bereinigtesHtml, titel);
  return rahmen;
}

/** Ziel eines Links (z. B. Hostname) für die Anzeige. */
function hostVon(adresse: string): string | null {
  try {
    const url = new URL(adresse);
    if (url.protocol === "mailto:") return null;
    return url.hostname.toLowerCase().replace(/^www\./, "");
  } catch {
    return null;
  }
}

const SIEHT_AUS_WIE_ADRESSE = /^(https?:\/\/)?([a-z0-9-]+\.)+[a-z]{2,}(:\d+)?([/?#]\S*)?$/i;
const SIEHT_AUS_WIE_MAIL = /^(mailto:)?[^\s@]+@[^\s@]+\.[^\s@]+$/i;

/**
 * Prüft einen Link. Gibt eine Warnung zurück, wenn der angezeigte Text eine andere
 * Adresse vortäuscht als das echte Ziel, oder `null`.
 */
export function linkPruefen(text: string, href: string): string | null {
  const angezeigt = text.trim();
  const ziel = href.trim();
  if (ziel.toLowerCase().startsWith("mailto:")) {
    if (SIEHT_AUS_WIE_MAIL.test(angezeigt)) {
      const textAdresse = angezeigt.replace(/^mailto:/i, "").toLowerCase();
      const zielAdresse = decodeURIComponent(ziel.slice(7).split("?")[0]).toLowerCase();
      if (textAdresse !== zielAdresse) {
        return `Der Link zeigt die Adresse „${angezeigt}“ an, schreibt aber an „${zielAdresse}“.`;
      }
    }
    return null;
  }
  const zielHost = hostVon(ziel);
  if (!zielHost) return null;
  if (SIEHT_AUS_WIE_ADRESSE.test(angezeigt)) {
    const textHost = hostVon(/^https?:\/\//i.test(angezeigt) ? angezeigt : `https://${angezeigt}`);
    if (textHost && textHost !== zielHost) {
      return `Der Link zeigt „${angezeigt}“ an, führt aber zu „${zielHost}“. Das ist typisch für Betrugsversuche (Phishing).`;
    }
  }
  if (zielHost.split(".").some((teil) => teil.startsWith("xn--"))) {
    return `Die Adresse „${zielHost}“ enthält Sonderzeichen, die bekannten Adressen zum Verwechseln ähnlich sehen können.`;
  }
  return null;
}

const URL_IM_TEXT = /\b(https?:\/\/[^\s<>"']+[^\s<>"'.,;:!?)\]])/gi;

/** Text mit anklickbaren Links (Links werden wie in HTML-Mails abgefangen). */
export function textMitLinks(text: string): DocumentFragment {
  const fragment = document.createDocumentFragment();
  let letztes = 0;
  for (const treffer of text.matchAll(URL_IM_TEXT)) {
    const start = treffer.index ?? 0;
    fragment.append(text.slice(letztes, start));
    const a = document.createElement("a");
    a.setAttribute("href", treffer[0]);
    a.textContent = treffer[0];
    fragment.append(a);
    letztes = start + treffer[0].length;
  }
  fragment.append(text.slice(letztes));
  return fragment;
}

/** Ein Klick auf einen Link: Ziel und angezeigter Text. */
export interface LinkKlick {
  href: string;
  text: string;
}

/** Sprungmarke, auf die Links im iframe umgeschrieben werden. */
export const LINK_MARKE = "#sichermail-link-";

/**
 * Bereitet die Links im iframe-Dokument vor.
 *
 * Im iframe laufen keine Skripte – auch keine Klick-Abfanglogik des Hauptfensters. Deshalb
 * wird jeder Link auf eine harmlose Sprungmarke (`about:blank#sichermail-link-<n>`) umgeschrieben.
 * Ein Klick lädt dann nur ein leeres Dokument im iframe; das Hauptfenster erkennt die Marke,
 * stellt den Inhalt wieder her und prüft den Link (siehe {@link linkAusNavigation}).
 * Beim Überfahren zeigt der Titel das echte Ziel.
 */
export function linksVorbereiten(dokument: Document): LinkKlick[] {
  const links: LinkKlick[] = [];
  for (const a of dokument.querySelectorAll("a")) {
    const href = a.getAttribute("href");
    a.removeAttribute("target");
    a.removeAttribute("ping");
    if (!href) continue;
    links.push({ href, text: a.textContent ?? "" });
    a.setAttribute("title", href);
    a.setAttribute("href", `about:blank${LINK_MARKE}${links.length - 1}`);
  }
  return links;
}

/** Ermittelt aus der Sprungmarke nach einer Navigation im iframe den geklickten Link. */
export function linkAusNavigation(hash: string, links: LinkKlick[]): LinkKlick | null {
  if (!hash.startsWith(LINK_MARKE)) return null;
  const index = Number.parseInt(hash.slice(LINK_MARKE.length), 10);
  return Number.isInteger(index) ? (links[index] ?? null) : null;
}

/**
 * Zeigt eine HTML-Mail im abgeschotteten iframe und leitet Link-Klicks an `beiKlick` weiter.
 * Jede andere Navigation im iframe wird sofort rückgängig gemacht.
 */
export function mailRahmenMitLinks(
  bereinigtesHtml: string,
  titel: string,
  beiKlick: (klick: LinkKlick) => void,
): HTMLIFrameElement {
  const rahmen = mailRahmen(bereinigtesHtml, titel);
  const dokument = rahmen.srcdoc;
  let links: LinkKlick[] = [];
  let inhaltErwartet = true;
  rahmen.addEventListener("load", () => {
    if (!inhaltErwartet) {
      // Das iframe hat navigiert: Marke auslesen, Inhalt wiederherstellen.
      let marke = "";
      try {
        marke = rahmen.contentWindow?.location.hash ?? "";
      } catch {
        marke = "";
      }
      const klick = linkAusNavigation(marke, links);
      inhaltErwartet = true;
      rahmen.srcdoc = dokument;
      if (klick) beiKlick(klick);
      return;
    }
    inhaltErwartet = false;
    const inhalt = rahmen.contentDocument;
    if (!inhalt) return;
    links = linksVorbereiten(inhalt);
    const hoehe = () => {
      rahmen.style.height = `${Math.max(inhalt.documentElement.scrollHeight, 80) + 4}px`;
    };
    hoehe();
    setTimeout(hoehe, 150);
  });
  return rahmen;
}

/**
 * Fängt Klicks auf Links in einem Bereich des Hauptfensters ab (z. B. Textansicht)
 * und zeigt beim Überfahren das echte Ziel. Für HTML im iframe siehe {@link mailRahmenMitLinks}.
 */
export function linksAbfangen(bereich: Document | HTMLElement, beiKlick: (klick: LinkKlick) => void): void {
  for (const a of bereich.querySelectorAll("a")) {
    const href = a.getAttribute("href");
    if (href) a.setAttribute("title", href);
  }
  bereich.addEventListener("click", (ereignis) => {
    const ziel = ereignis.target as Element | null;
    const a = ziel?.closest?.("a");
    if (!a) return;
    ereignis.preventDefault();
    const href = a.getAttribute("href");
    if (href) beiKlick({ href, text: a.textContent ?? "" });
  });
  // Formulare gibt es nach der Bereinigung nicht; falls doch, nie absenden.
  bereich.addEventListener("submit", (ereignis) => ereignis.preventDefault());
}

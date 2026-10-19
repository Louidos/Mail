/** Baut die Grundoberfläche von SicherMail auf. */
export function appStarten(wurzel: HTMLElement): void {
  wurzel.replaceChildren();
  const titel = document.createElement("h1");
  titel.textContent = "SicherMail";
  const hinweis = document.createElement("p");
  hinweis.textContent = "Noch kein Konto eingerichtet.";
  wurzel.append(titel, hinweis);
}

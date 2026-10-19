import { appStarten } from "./app";
import { darstellungStarten } from "./einstellungen/darstellung";

// Sofort, nicht erst nach dem Laden der Seite: Die Darstellung soll vor dem ersten Zeichnen stehen.
void darstellungStarten();

window.addEventListener("DOMContentLoaded", () => {
  const wurzel = document.getElementById("app");
  if (wurzel) {
    appStarten(wurzel);
  }
});

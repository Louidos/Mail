import { appStarten } from "./app";

window.addEventListener("DOMContentLoaded", () => {
  const wurzel = document.getElementById("app");
  if (wurzel) {
    appStarten(wurzel);
  }
});

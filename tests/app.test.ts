import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { appStarten } = await import("../src/app");

const warten = () => new Promise((r) => setTimeout(r, 0));

describe("appStarten", () => {
  beforeEach(() => {
    invoke.mockReset();
    document.body.replaceChildren();
  });

  it("zeigt den Programmnamen und ohne Konten eine Begrüssung", async () => {
    invoke.mockResolvedValue([]);
    const wurzel = document.createElement("div");
    appStarten(wurzel);
    await warten();
    expect(wurzel.querySelector("h1")?.textContent).toBe("SicherMail");
    expect(wurzel.textContent).toContain("Willkommen bei SicherMail");
    expect(invoke).toHaveBeenCalledWith("konten_auflisten");
  });

  it("listet vorhandene Konten auf", async () => {
    invoke.mockResolvedValue([
      {
        id: "1",
        name: "Anna",
        email: "anna@example.org",
        benutzername: "anna",
        imap: { host: "imap.example.org", port: 993, verschluesselung: "tls" },
        smtp: { host: "smtp.example.org", port: 465, verschluesselung: "tls" },
      },
    ]);
    const wurzel = document.createElement("div");
    appStarten(wurzel);
    await warten();
    const eintraege = wurzel.querySelectorAll("li.konto");
    expect(eintraege).toHaveLength(1);
    expect(eintraege[0].textContent).toContain("anna@example.org");
  });

  it("setzt Texte nie als HTML", async () => {
    invoke.mockResolvedValue([
      {
        id: "1",
        name: "<img src=x>",
        email: "a@example.org",
        benutzername: "a",
        imap: { host: "i.example.org", port: 993, verschluesselung: "tls" },
        smtp: { host: "s.example.org", port: 465, verschluesselung: "tls" },
      },
    ]);
    const wurzel = document.createElement("div");
    appStarten(wurzel);
    await warten();
    expect(wurzel.querySelector("img")).toBeNull();
    expect(wurzel.textContent).toContain("<img src=x>");
  });
});

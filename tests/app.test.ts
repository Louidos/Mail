import { describe, expect, it } from "vitest";
import { appStarten } from "../src/app";

describe("appStarten", () => {
  it("zeigt den Programmnamen an", () => {
    const wurzel = document.createElement("div");
    appStarten(wurzel);
    expect(wurzel.querySelector("h1")?.textContent).toBe("SicherMail");
  });
});

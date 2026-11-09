/*
 * semtext-mock-web.js — dependency-free custom element for the smah web
 * virtual tool page (#19). Presentation only: it re-fetches content.json on
 * driver regeneration events and re-renders the page.
 *
 * Security (THREAT-MODEL T-019): every fabricated string is assigned via
 * textContent — never innerHTML and never insertAdjacentHTML — so
 * model-generated content can never execute as markup. The push channel
 * (/push, Server-Sent Events) is origin-checked server-side and connect-src
 * 'self' in the page CSP.
 */
"use strict";

class SemtextPage extends HTMLElement {
  connectedCallback() {
    this.refresh();
    this.watch();
  }

  async refresh() {
    let obj;
    try {
      const res = await fetch("/content.json", { credentials: "omit" });
      if (!res.ok) return;
      obj = await res.json();
    } catch {
      // Server restarting between driver regenerations; the next push event
      // triggers another refresh.
      return;
    }
    if (obj === null || typeof obj !== "object" || Array.isArray(obj)) return;

    const title = document.getElementById("page-title");
    if (title && typeof obj.title === "string") {
      title.textContent = obj.title;
    }

    const main = document.getElementById("sections");
    if (!main) return;
    const notice = main.querySelector(".notice");
    main.replaceChildren();
    if (notice) main.append(notice);

    const keys = Object.keys(obj)
      .filter((k) => k !== "title")
      .sort();
    for (const key of keys) {
      const value = obj[key];
      if (typeof value !== "string") continue;
      const sec = document.createElement("section");
      const h = document.createElement("h2");
      h.textContent = key;
      const p = document.createElement("p");
      p.textContent = value;
      sec.append(h, p);
      main.append(sec);
    }
  }

  watch() {
    if (!("EventSource" in window)) return;
    const es = new EventSource("/push");
    es.addEventListener("content", () => this.refresh());
  }
}

customElements.define("semtext-page", SemtextPage);

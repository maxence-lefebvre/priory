"use strict";

// mdBook's searcher.js calls this book.js helper, which this theme does not load.
function mdbook_something_else_has_focus(e) {
  const target = e.composedPath()[0] || e.target;
  return /^(?:input|select|textarea)$/i.test(target.nodeName);
}

(function () {
  const html = document.documentElement;
  const dark = window.matchMedia("(prefers-color-scheme: dark)");

  // Light and dark: follow the system until the person picks one, then keep it on this device.
  const modeButton = document.getElementById("mode-toggle");
  const isDark = () => html.dataset.mode === "dark" || (html.dataset.mode === "system" && dark.matches);
  const labelMode = () => {
    modeButton.setAttribute("aria-label", isDark() ? "Switch to light mode" : "Switch to dark mode");
  };
  modeButton.addEventListener("click", () => {
    html.dataset.mode = isDark() ? "light" : "dark";
    try { localStorage.setItem("priory-docs-mode", html.dataset.mode); } catch (e) {}
    labelMode();
  });
  dark.addEventListener("change", labelMode);
  labelMode();

  // The navigation opens as a full sheet below 900 px.
  // While it is open, everything it covers is inert, so focus stays in the sheet.
  const navButton = document.getElementById("nav-toggle");
  const nav = document.getElementById("mdbook-sidebar");
  const covered = [document.getElementById("mdbook-content"), document.querySelector("footer"), document.querySelector(".toc")];
  const compact = window.matchMedia("(max-width: 899px)");
  const setNav = (open) => {
    html.classList.toggle("nav-open", open);
    covered.forEach((el) => { if (el) el.inert = open; });
    navButton.setAttribute("aria-expanded", String(open));
    navButton.setAttribute("aria-label", open ? "Close the navigation" : "Open the navigation");
    const current = nav.querySelector("a.active") || nav.querySelector("a");
    if (open && current) {
      current.focus({ preventScroll: true });
      current.scrollIntoView({ block: "center" });
    }
  };
  navButton.addEventListener("click", () => setNav(!html.classList.contains("nav-open")));
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !html.classList.contains("nav-open")) return;
    setNav(false);
    navButton.focus();
  });
  compact.addEventListener("change", () => { if (!compact.matches) setNav(false); });

  // The search index loads on first focus, without the jump to the top that mdBook's toggle does.
  const searchbar = document.getElementById("mdbook-searchbar");
  const searchWrap = document.getElementById("mdbook-search-wrapper");
  const searchToggle = document.getElementById("mdbook-search-toggle");
  if (searchbar && searchToggle) {
    searchbar.addEventListener("focus", () => {
      if (!searchWrap.classList.contains("hidden")) return;
      const y = window.scrollY;
      searchToggle.click();
      window.scrollTo(0, y);
    });
  }

  // Copy buttons on code blocks.
  document.querySelectorAll("main pre").forEach((pre) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "iconbtn copy";
    button.setAttribute("aria-label", "Copy the code");
    button.innerHTML = '<svg class="i" viewBox="0 0 24 24" aria-hidden="true"><rect width="14" height="14" x="8" y="8" rx="2"></rect><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"></path></svg>';
    button.addEventListener("click", () => {
      const say = (label) => {
        button.setAttribute("aria-label", label);
        setTimeout(() => button.setAttribute("aria-label", "Copy the code"), 2000);
      };
      navigator.clipboard.writeText(pre.querySelector("code").textContent)
        .then(() => say("Copied"), () => say("Could not copy: the browser blocked the clipboard"));
    });
    pre.prepend(button);
  });

  // Section tabs: the current one holds the active sidebar link's group.
  const activeLink = document.querySelector("#mdbook-sidebar a.active");
  let groupItem = activeLink && activeLink.closest("li");
  while (groupItem && !groupItem.classList.contains("part-title")) groupItem = groupItem.previousElementSibling;
  const group = groupItem ? groupItem.textContent.trim() : null;
  document.querySelectorAll(".tabs a").forEach((tab) => {
    if (group && tab.dataset.groups.split("|").includes(group)) tab.setAttribute("aria-current", "true");
  });

  // Copy as Markdown: build.py writes each page's source beside it as <page>.md.txt.
  const copyMarkdown = document.querySelector(".copy-md");
  if (copyMarkdown && navigator.clipboard) {
    const label = copyMarkdown.querySelector("span");
    const say = (text) => {
      label.textContent = text;
      setTimeout(() => { label.textContent = "Copy as Markdown"; }, 2000);
    };
    const source = () => fetch(copyMarkdown.dataset.markdown).then((r) => {
      if (!r.ok) throw new Error(r.statusText);
      return r.text();
    });
    copyMarkdown.addEventListener("click", () => {
      // Safari keeps the click's permission only when the clipboard write starts before the fetch ends.
      const write = window.ClipboardItem
        ? navigator.clipboard.write([new ClipboardItem({ "text/plain": source().then((t) => new Blob([t], { type: "text/plain" })) })])
        : source().then((t) => navigator.clipboard.writeText(t));
      write.then(() => say("Copied"), () => say("Could not copy the page"));
    });
    copyMarkdown.hidden = false;
  }

  if (html.classList.contains("is-home")) return;

  // Breadcrumb and page kind from the active sidebar link's group.
  const KINDS = {
    "Start": "Guide",
    "Reference": "Reference",
    "The daemon": "How it works",
    "The intake": "How it works",
    "Adapters": "How it works",
    "Decisions": "Decision",
    "Architecture decisions": "Decision",
    "Design notes": "Explanation",
  };
  const active = activeLink;
  if (group) {
    document.getElementById("crumb-group").textContent = group;
    const kind = document.getElementById("page-kind");
    if (KINDS[group]) {
      kind.textContent = KINDS[group];
      kind.hidden = false;
    }
  }
  if (active) active.setAttribute("aria-current", "page");

  // Tables: a box that scrolls on its own, and reference tables that stack on a phone.
  const REF = "key,type,required,default,meaning,example";
  document.querySelectorAll(".prose table").forEach((table) => {
    let box = table.parentElement;
    if (!box.classList.contains("table-wrapper")) {
      box = document.createElement("div");
      table.replaceWith(box);
      box.append(table);
    }
    box.classList.add("tbl");
    const heads = [...table.querySelectorAll("thead th")].map((th) => th.textContent.trim().toLowerCase());
    if (heads.join(",") !== REF) return;
    box.classList.add("ref");
    table.querySelectorAll("tbody tr").forEach((row) => {
      const required = row.children[2];
      if (required && required.textContent.trim() === "no") required.classList.add("opt");
    });
  });

  // On this page: the h2 and h3 headings, on the right from 1200 px and folded under the title below.
  const headings = [...document.querySelectorAll(".prose h2[id], .prose h3[id]")];
  if (headings.length < 2) return;
  const entry = (h) => {
    const li = document.createElement("li");
    if (h.tagName === "H3") li.className = "sub";
    const a = document.createElement("a");
    a.href = `#${h.id}`;
    a.append(h.textContent.trim());
    li.append(a);
    return li;
  };
  const list = document.getElementById("toc-list");
  headings.forEach((h) => list.append(entry(h)));
  list.closest(".toc").hidden = false;

  const inline = document.createElement("details");
  inline.className = "toc-inline";
  inline.innerHTML = '<summary><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 12h.01M3 18h.01M3 6h.01M8 12h13M8 18h13M8 6h13"></path></svg>On this page</summary><ul></ul>';
  headings.filter((h) => h.tagName === "H2").forEach((h) => inline.querySelector("ul").append(entry(h)));
  const title = document.querySelector(".prose h1");
  if (title) title.after(inline);
  else document.querySelector(".prose").prepend(inline);

  const links = [...list.querySelectorAll("a")];
  const markCurrent = () => {
    const line = 120;
    let current = 0;
    headings.forEach((h, i) => { if (h.getBoundingClientRect().top < line) current = i; });
    if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 2) current = headings.length - 1;
    links.forEach((a, i) => {
      if (i === current) a.setAttribute("aria-current", "true");
      else a.removeAttribute("aria-current");
    });
  };
  document.addEventListener("scroll", markCurrent, { passive: true });
  markCurrent();
})();

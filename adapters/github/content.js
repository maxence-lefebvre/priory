// In GitHub's page: places the frames and the Guide tab and reads the guide block; never the token.
(async () => {
  const { parsePr, headSha } = await import(chrome.runtime.getURL("lib/pr.js"));
  const { anchor, blockIn, fallback, tabLike } = await import(chrome.runtime.getURL("lib/page.js"));
  const FRAME = chrome.runtime.getURL("frame.html");
  const WIDTH = 420;
  let shown = null;
  let strip = null;
  let panel = null;
  let guideTab = null;
  let guide = null;
  let warned = null;
  let warnTimer = 0;
  const SETTLE_MS = 3000;

  const mode = () => ({ light: "light", dark: "dark" })[document.documentElement.dataset.colorMode] ?? "system";
  const src = (view, tab = "") => `${FRAME}?view=${view}&mode=${mode()}${tab && `&tab=${tab}`}`;

  const frame = (view, style) => {
    const element = document.createElement("iframe");
    element.src = src(view);
    element.title = "Priory";
    element.setAttribute("style", `border:0;color-scheme:normal;${style}`);
    return element;
  };

  // The strip goes above the tab row. Until one shows, it goes at the top of the page, so a page
  // GitHub changed still gets the strip, and one warning names the page if no row shows in time.
  function place() {
    const found = anchor(document);
    strip ??= frame("strip", "display:block;width:100%;height:50px;margin:0 0 8px");
    if (found && (!strip.isConnected || strip.dataset.at === "fallback")) {
      strip.dataset.at = found.kind;
      found.nav.before(strip);
    } else if (!found && !strip.isConnected) {
      strip.dataset.at = "fallback";
      fallback(document).prepend(strip);
    }
    if (!found) {
      if (warned !== shown?.url && !warnTimer) warnTimer = setTimeout(warn, SETTLE_MS);
      return;
    }
    if (guide?.count && !guideTab?.isConnected) {
      const like = tabLike(found.nav);
      if (!like) return;
      guideTab = like.cloneNode(false);
      for (const name of ["aria-current", "aria-selected", "data-turbo-frame", "id", "aria-controls"]) guideTab.removeAttribute(name);
      guideTab.classList.remove("selected");
      if (guideTab.localName === "a") guideTab.href = "#priory-guide";
      guideTab.textContent = `Guide ${guide.count}`;
      guideTab.addEventListener("click", (event) => {
        event.preventDefault();
        open("guide");
      });
      like.parentElement.append(guideTab);
    }
  }

  function warn() {
    warnTimer = 0;
    if (!shown || anchor(document) || warned === shown.url) return;
    warned = shown.url;
    console.warn(`Priory: no tab row found on ${location.href}, so the strip sits at the top of the page. Please report this page.`);
  }

  function open(tab) {
    if (!panel) {
      panel = frame("panel", `position:fixed;top:0;right:0;width:${WIDTH}px;height:100vh;z-index:2147483646;box-shadow:0 0 0 1px #8885`);
      document.documentElement.append(panel);
      document.body.style.marginRight = `${WIDTH}px`;
    }
    panel.src = src("panel", tab);
  }

  function close() {
    panel?.remove();
    panel = null;
    document.body.style.marginRight = "";
  }

  // Fetched each time, never read from this page: after a push or an edit, GitHub updates the page
  // in place but not the head sha in its embedded data.
  async function readGuide(pr) {
    try {
      const response = await fetch(pr.url, { credentials: "same-origin", cache: "no-store" });
      if (!response.ok) return { failed: true };
      const html = await response.text();
      return { json: blockIn(new DOMParser().parseFromString(html, "text/html")), head: headSha(html) };
    } catch {
      return { failed: true };
    }
  }

  async function mount(pr) {
    const read = await readGuide(pr);
    if (shown !== pr) return;
    guide = read;
    const count = guide.json?.match(/"question"\s*:/g)?.length;
    guide.count = guide.json ? count || "" : 0;
    place();
  }

  function unmount() {
    for (const element of [strip, guideTab]) element?.remove();
    close();
    clearTimeout(warnTimer);
    warnTimer = 0;
    strip = guideTab = guide = null;
  }

  async function sync() {
    const pr = parsePr(location.href);
    if (pr?.url !== shown?.url) {
      unmount();
      shown = pr;
      if (pr) await mount(pr);
      return;
    }
    if (pr) place();
  }

  chrome.runtime.onMessage.addListener((message, sender, respond) => {
    if (sender.id !== chrome.runtime.id || !sender.url?.startsWith(FRAME)) return;
    if (message.type === "guide") {
      const pr = shown;
      (pr ? readGuide(pr) : Promise.resolve({ failed: true })).then((read) =>
        respond({ pr, json: read.json ?? null, head: read.head ?? null, failed: Boolean(read.failed) }));
      return true;
    }
    if (message.type === "open") open(message.tab);
    if (message.type === "close") close();
  });

  let pending = 0;
  new MutationObserver(() => {
    clearTimeout(pending);
    pending = setTimeout(sync, 150);
  }).observe(document.body, { childList: true, subtree: true });
  sync();
})();

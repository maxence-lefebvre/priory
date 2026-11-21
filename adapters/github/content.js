// Runs in GitHub's page. It finds the pull request, places the extension's two frames and the
// Guide tab, and reads the guide block. It never holds the token, calls the daemon or shows agent
// text: the frames do, in the extension's origin.
(async () => {
  const { parsePr, headSha } = await import(chrome.runtime.getURL("lib/pr.js"));
  const FRAME = chrome.runtime.getURL("frame.html");
  const WIDTH = 420;
  let shown = null;
  let strip = null;
  let panel = null;
  let guideTab = null;
  let guide = null;

  const mode = () => ({ light: "light", dark: "dark" })[document.documentElement.dataset.colorMode] ?? "system";
  const src = (view, tab = "") => `${FRAME}?view=${view}&mode=${mode()}${tab && `&tab=${tab}`}`;

  const frame = (view, style) => {
    const element = document.createElement("iframe");
    element.src = src(view);
    element.title = "Priory";
    element.setAttribute("style", `border:0;color-scheme:normal;${style}`);
    return element;
  };

  const prNav = () => document.querySelector('nav[aria-label="Pull request navigation"], .tabnav-pr, .tabnav');

  function place() {
    const nav = prNav();
    if (!nav) return;
    if (!strip?.isConnected) {
      strip ??= frame("strip", "display:block;width:100%;height:50px;margin:0 0 8px");
      nav.before(strip);
    }
    if (guide?.count && !guideTab?.isConnected) {
      const like = nav.querySelector("a");
      if (!like) return;
      guideTab = like.cloneNode(false);
      guideTab.removeAttribute("aria-current");
      guideTab.removeAttribute("data-turbo-frame");
      guideTab.classList.remove("selected");
      guideTab.href = "#priory-guide";
      guideTab.textContent = `Guide ${guide.count}`;
      guideTab.addEventListener("click", (event) => {
        event.preventDefault();
        open("guide");
      });
      like.parentElement.append(guideTab);
    }
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

  // The description holds the block as GitHub renders it: a <details> titled "Review guide"
  // around the JSON. GitHub drops the HTML comment markers from the page.
  function blockIn(doc) {
    const body = doc.querySelector(".js-comment-body, .comment-body");
    const details = [...(body?.querySelectorAll("details") ?? [])].find((element) => element.querySelector("summary")?.textContent.trim() === "Review guide");
    return details?.querySelector("pre")?.textContent ?? null;
  }

  async function readGuide(pr) {
    try {
      if (pr.tab === "conversation" && document.querySelector(".js-comment-body, .comment-body")) {
        return { json: blockIn(document), head: headSha(document.documentElement.innerHTML) };
      }
      const response = await fetch(pr.url, { credentials: "same-origin" });
      if (!response.ok) return { failed: true };
      const html = await response.text();
      return { json: blockIn(new DOMParser().parseFromString(html, "text/html")), head: headSha(html) };
    } catch {
      return { failed: true };
    }
  }

  async function mount(pr) {
    guide = await readGuide(pr);
    const count = guide.json?.match(/"question"\s*:/g)?.length;
    guide.count = guide.json ? count || "" : 0;
    place();
  }

  function unmount() {
    for (const element of [strip, guideTab]) element?.remove();
    close();
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
      (message.retry && shown ? readGuide(shown).then((read) => Object.assign(guide, read)) : Promise.resolve()).then(() =>
        respond({ pr: shown, json: guide?.json ?? null, head: guide?.head ?? null, failed: Boolean(guide?.failed) }));
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

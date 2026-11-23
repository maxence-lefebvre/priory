// A small DOM for the tests: parses HTML and answers the calls the extension's scripts make.
// It covers only what content.js, frame.js and their fixtures use.

const VOID = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"]);
const RAW = new Set(["script", "style", "textarea", "title"]);
const ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
const decode = (text) =>
  text.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (whole, name) =>
    name[0] === "#" ? String.fromCodePoint(name[1].toLowerCase() === "x" ? parseInt(name.slice(2), 16) : Number(name.slice(1))) : ENTITIES[name] ?? whole);

export class Node {
  constructor(document) {
    this.ownerDocument = document;
    this.parentNode = null;
    this.childNodes = [];
  }
  get parentElement() {
    return this.parentNode instanceof Element ? this.parentNode : null;
  }
  get isConnected() {
    let node = this;
    while (node.parentNode) node = node.parentNode;
    return node === this.ownerDocument;
  }
  get textContent() {
    return this.childNodes.map((child) => child.textContent).join("");
  }
  set textContent(text) {
    this.replaceChildren(String(text));
  }
  #adopt(kids) {
    return kids.map((kid) => {
      const node = kid instanceof Node ? kid : new Text(this.ownerDocument, String(kid));
      node.remove();
      node.parentNode = this;
      return node;
    });
  }
  append(...kids) {
    this.childNodes.push(...this.#adopt(kids));
  }
  prepend(...kids) {
    this.childNodes.unshift(...this.#adopt(kids));
  }
  replaceChildren(...kids) {
    for (const child of this.childNodes) child.parentNode = null;
    this.childNodes = [];
    this.append(...kids);
  }
  before(...kids) {
    const parent = this.parentNode;
    if (!parent) return;
    const nodes = parent.#adopt(kids);
    parent.childNodes.splice(parent.childNodes.indexOf(this), 0, ...nodes);
  }
  after(...kids) {
    const parent = this.parentNode;
    if (!parent) return;
    const nodes = parent.#adopt(kids);
    parent.childNodes.splice(parent.childNodes.indexOf(this) + 1, 0, ...nodes);
  }
  remove() {
    if (!this.parentNode) return;
    const siblings = this.parentNode.childNodes;
    siblings.splice(siblings.indexOf(this), 1);
    this.parentNode = null;
  }
  get children() {
    return this.childNodes.filter((child) => child instanceof Element);
  }
  *descendants() {
    for (const child of this.children) {
      yield child;
      yield* child.descendants();
    }
  }
  querySelectorAll(selector) {
    const parsed = parseSelector(selector);
    return [...this.descendants()].filter((element) => parsed.some((chain) => matches(element, chain)));
  }
  querySelector(selector) {
    return this.querySelectorAll(selector)[0] ?? null;
  }
}

export class Text extends Node {
  constructor(document, data) {
    super(document);
    this.data = data;
  }
  get textContent() {
    return this.data;
  }
  cloneNode() {
    return new Text(this.ownerDocument, this.data);
  }
}

export class Element extends Node {
  constructor(document, tag) {
    super(document);
    this.localName = tag.toLowerCase();
    this.attributes = new Map();
    this.listeners = [];
    this.value = "";
    this.disabled = false;
    this.style = {};
    const element = this;
    this.dataset = new Proxy({}, {
      get: (_, key) => element.getAttribute(`data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`) ?? undefined,
      set: (_, key, value) => (element.setAttribute(`data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`, value), true),
    });
    this.classList = {
      contains: (name) => element.className.split(/\s+/).includes(name),
      add: (name) => element.classList.contains(name) || element.setAttribute("class", `${element.className} ${name}`.trim()),
      remove: (name) => element.setAttribute("class", element.className.split(/\s+/).filter((c) => c && c !== name).join(" ")),
    };
  }
  get tagName() {
    return this.localName.toUpperCase();
  }
  get id() {
    return this.getAttribute("id") ?? "";
  }
  get className() {
    return this.getAttribute("class") ?? "";
  }
  get href() {
    return this.getAttribute("href") ?? "";
  }
  set href(value) {
    this.setAttribute("href", value);
  }
  get src() {
    return this.getAttribute("src") ?? "";
  }
  set src(value) {
    this.setAttribute("src", value);
  }
  get title() {
    return this.getAttribute("title") ?? "";
  }
  set title(value) {
    this.setAttribute("title", value);
  }
  getAttribute(name) {
    return this.attributes.has(name) ? this.attributes.get(name) : null;
  }
  setAttribute(name, value) {
    this.attributes.set(name.toLowerCase(), String(value));
  }
  removeAttribute(name) {
    this.attributes.delete(name);
  }
  hasAttribute(name) {
    return this.attributes.has(name);
  }
  closest(selector) {
    const parsed = parseSelector(selector);
    for (let element = this; element; element = element.parentElement) if (parsed.some((chain) => matches(element, chain))) return element;
    return null;
  }
  matches(selector) {
    return parseSelector(selector).some((chain) => matches(this, chain));
  }
  cloneNode(deep = false) {
    const copy = new Element(this.ownerDocument, this.localName);
    for (const [name, value] of this.attributes) copy.attributes.set(name, value);
    if (deep) copy.append(...this.childNodes.map((child) => child.cloneNode(true)));
    return copy;
  }
  addEventListener(type, listener) {
    this.listeners.push([type, listener]);
  }
  removeEventListener(type, listener) {
    this.listeners = this.listeners.filter(([t, l]) => t !== type || l !== listener);
  }
  /** Runs the listeners of `type` here and up the tree, as a bubbling event would. */
  dispatch(type, init = {}) {
    const event = { type, target: this, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; }, ...init };
    for (let element = this; element; element = element.parentElement) {
      event.currentTarget = element;
      for (const [t, listener] of element.listeners) if (t === type) listener(event);
    }
    return event;
  }
  click() {
    return this.dispatch("click");
  }
  focus() {
    this.ownerDocument.activeElement = this;
  }
  scrollTo() {}
  setSelectionRange() {}
}

export class Document extends Node {
  constructor() {
    super(null);
    this.ownerDocument = this;
    this.activeElement = null;
  }
  get documentElement() {
    return this.children[0] ?? null;
  }
  get body() {
    return this.querySelector("body");
  }
  createElement(tag) {
    return new Element(this, tag);
  }
  createElementNS(_, tag) {
    return new Element(this, tag);
  }
  createTextNode(data) {
    return new Text(this, data);
  }
  getElementById(id) {
    return this.querySelector(`#${id}`);
  }
  get isConnected() {
    return true;
  }
}

/** A document of `html`, with an <html> and a <body> when the markup has none. */
export function parse(html) {
  const document = new Document();
  const stack = [document];
  const top = () => stack[stack.length - 1];
  const tags = /<!--[\s\S]*?-->|<!doctype[^>]*>|<\/([a-zA-Z][\w-]*)\s*>|<([a-zA-Z][\w-]*)((?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)*)\s*(\/?)>/gi;
  let at = 0;
  for (let match; (match = tags.exec(html)); ) {
    if (match.index > at) top().append(new Text(document, decode(html.slice(at, match.index))));
    at = tags.lastIndex;
    const [, close, open, attrs, self] = match;
    if (close) {
      const name = close.toLowerCase();
      const index = stack.findLastIndex((node) => node.localName === name);
      if (index > 0) stack.length = index;
      continue;
    }
    if (!open) continue;
    const element = new Element(document, open);
    for (const [, name, , dq, sq, bare] of attrs.matchAll(/([^\s=/>]+)(\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g)) {
      element.attributes.set(name.toLowerCase(), decode(dq ?? sq ?? bare ?? ""));
    }
    top().append(element);
    if (self || VOID.has(element.localName)) continue;
    if (RAW.has(element.localName)) {
      const end = html.toLowerCase().indexOf(`</${element.localName}`, at);
      const stop = end < 0 ? html.length : end;
      element.append(new Text(document, html.slice(at, stop)));
      tags.lastIndex = at = html.indexOf(">", stop) + 1 || html.length;
      continue;
    }
    stack.push(element);
  }
  if (at < html.length) top().append(new Text(document, decode(html.slice(at))));
  if (!document.querySelector("html")) {
    const root = new Element(document, "html");
    const body = new Element(document, "body");
    body.append(...[...document.childNodes]);
    root.append(body);
    document.append(root);
  } else if (!document.querySelector("body")) {
    document.documentElement.append(new Element(document, "body"));
  }
  return document;
}

export class DOMParser {
  parseFromString(text) {
    return parse(text);
  }
}

// Selectors: lists of chains of compound parts, joined by descendant or child combinators.
function parseSelector(selector) {
  const lists = [];
  let depth = 0;
  let quote = null;
  let start = 0;
  for (let i = 0; i < selector.length; i += 1) {
    const c = selector[i];
    if (quote) quote = c === quote ? null : quote;
    else if (c === '"' || c === "'") quote = c;
    else if (c === "[") depth += 1;
    else if (c === "]") depth -= 1;
    else if (c === "," && depth === 0) {
      lists.push(selector.slice(start, i));
      start = i + 1;
    }
  }
  lists.push(selector.slice(start));
  return lists.map((text) => {
    const chain = [];
    let combinator = " ";
    for (const token of text.trim().match(/(?:[^\s>"'[\]]+|\[[^\]]*\])+|>/g) ?? []) {
      if (token === ">") {
        combinator = ">";
        continue;
      }
      chain.push({ combinator, test: compound(token) });
      combinator = " ";
    }
    return chain;
  });
}

function compound(token) {
  const tests = [];
  for (const [part] of token.matchAll(/\[[^\]]*\]|[#.]?[\w-]+|\*/g)) {
    if (part === "*") continue;
    if (part[0] === "#") tests.push((element) => element.id === part.slice(1));
    else if (part[0] === ".") tests.push((element) => element.classList.contains(part.slice(1)));
    else if (part[0] === "[") {
      const [, name, op, raw] = part.match(/^\[\s*([\w-]+)\s*(?:([~^$*|]?=)\s*(.*?))?\s*\]$/);
      const value = raw?.replace(/^["']|["']$/g, "");
      tests.push((element) => {
        const have = element.getAttribute(name);
        if (have == null) return false;
        if (!op) return true;
        if (op === "=") return have === value;
        if (op === "^=") return have.startsWith(value);
        if (op === "$=") return have.endsWith(value);
        if (op === "*=") return have.includes(value);
        if (op === "~=") return have.split(/\s+/).includes(value);
        return have === value || have.startsWith(`${value}-`);
      });
    } else tests.push((element) => element.localName === part.toLowerCase());
  }
  return (element) => tests.every((test) => test(element));
}

function matches(element, chain, index = chain.length - 1) {
  if (!chain[index].test(element)) return false;
  if (index === 0) return true;
  if (chain[index].combinator === ">") return element.parentElement ? matches(element.parentElement, chain, index - 1) : false;
  for (let up = element.parentElement; up; up = up.parentElement) if (matches(up, chain, index - 1)) return true;
  return false;
}

// How the extension reaches `priory serve`: the token goes only in the Authorization header, only
// to the origin the person gave, and only on a path under /api.

// The manifest's host_permissions cover these; any other address must be https.
const LOOPBACK = new Set(["127.0.0.1", "localhost"]);

/** The origin of the address the person typed, or why it is refused. @param {string} input */
export function address(input) {
  const text = input.trim();
  if (!text) return { error: "Enter the address priory serve listens on." };
  let url;
  try {
    url = new URL(/^[a-z]+:\/\//i.test(text) ? text : `${LOOPBACK.has(text.split(":")[0]) ? "http" : "https"}://${text}`);
  } catch {
    return { error: "This address does not parse." };
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return { error: "Use an http or https address." };
  if (url.protocol === "http:" && !LOOPBACK.has(url.hostname)) {
    return { error: "Use https beyond this machine, such as tailscale serve gives, so the token never travels in clear." };
  }
  if (url.username || url.password || url.pathname !== "/" || url.search || url.hash) {
    return { error: "Enter the address alone, with no path, query or name." };
  }
  return { origin: url.origin };
}

/** The URL and fetch options of a call: a GET, or a POST of `body` as JSON. */
export function request(origin, token, path, body) {
  if (!path.startsWith("/api/")) throw new Error(`the path ${path} is not under /api`);
  const headers = { Authorization: `Bearer ${token}` };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  return {
    url: origin + path,
    init: {
      method: body === undefined ? "GET" : "POST",
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: "no-store",
      credentials: "omit",
      redirect: "error",
    },
  };
}

/**
 * Sends a call and reads its reply as `{ status, body }`. A read that fails on the network is
 * `unreachable`; a write that did, or got 504, is `unknown`: the daemon may have applied it.
 */
export async function call(fetcher, origin, token, path, body, timeout = 15000) {
  const { url, init } = request(origin, token, path, body);
  const write = body !== undefined;
  try {
    const response = await fetcher(url, { ...init, signal: AbortSignal.timeout(timeout) });
    const json = await response.json().catch(() => ({}));
    return write && response.status === 504 ? { status: 504, body: json, unknown: true } : { status: response.status, body: json };
  } catch {
    return write ? { status: 0, body: {}, unknown: true } : { status: 0, body: {}, unreachable: true };
  }
}

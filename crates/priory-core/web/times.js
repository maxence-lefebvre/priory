// Times: relative under 24 h, then a date (DESIGN.md, Copy).
export function age(iso) {
  if (!iso) return "";
  const elapsed = Date.now() - Date.parse(iso);
  if (elapsed < 60000) return "just now";
  const minutes = Math.round(elapsed / 60000);
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`;
  return date(iso);
}
export const date = (iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false });
export const clock = (iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hour12: false });
export const today = (iso) => new Date(iso).toDateString() === new Date().toDateString();
export const ago = (iso) => {
  const text = age(iso);
  if (text === "just now") return text;
  return / (min|h)$/.test(text) ? `${text} ago` : `on ${text}`;
};

// The desktop shortcuts of every page: J and K move, Enter opens, Esc closes, N adds. Never inside a
// field or under a sheet. `keys` are the entries' own, `at` the place on screen, and `rows` the asks
// the list draws, each { href, current },
// and `onBody` whether no control has the focus.
export function shortcut(event, { at, rows, onBody, go, add, keys }) {
  const move = (to) => to && go(to.href);
  const current = rows.findIndex((row) => row.current);
  if (event.key === "n" || event.key === "N") {
    event.preventDefault();
    add();
  } else if (keys(event)) {
    return;
  } else if (event.key === "j") move(rows[Math.min(rows.length - 1, current + 1)]);
  else if (event.key === "k") move(rows[Math.max(0, current - 1)]);
  else if (event.key === "Enter" && current >= 0 && onBody) move(rows[current]);
  else if (event.key === "Escape" && at.ask) go(at.listUrl);
}

// What the page reads once the revision moves, sorted. Appearance's theme previews draw the Desk.
export const reads = [
  ["/settings/appearance", "/api/asks /api/desk /api/queue /api/tasks"],
  ["/settings/notifications", "/api/desk"],
];

// Every theme's two previews draw the Desk, opened with no copy kept on this device, and draw it
// again once the revision moves.
const DRAWN = `[...document.querySelectorAll(".theme .preview > div")].filter((part) => part.querySelector(".panel .scroll")).length`;
export async function check({ hooked, evaluate, until, change }) {
  await hooked("each theme previews the Desk, light and dark, cold and through a refresh", "/settings/appearance", 'document.querySelector(".themes")', { cold: true }, async () => {
    await until(`${DRAWN} === 10`, "the theme previews");
    await evaluate('document.querySelector(".theme .preview > div").innerHTML = ""');
    change();
    await until(`${DRAWN} === 10`, "the previews drawn again");
    return true;
  });
}

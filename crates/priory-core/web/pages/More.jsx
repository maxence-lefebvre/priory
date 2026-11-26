// More (DESIGN.md, More): a phone's panels past the first four, Settings and the sign-out.
import { memo } from "react";
import { Icon } from "../components/Icon.jsx";
import { leaves } from "../frame.js";
import { PANELS } from "../pages.js";
import { signOut } from "../session.js";
import { listClass, usePlace } from "./place.js";

const Link = ({ href, icon, children }) => (
  <li><a href={href}><Icon name={icon} size="s20" />{children}<span className="chev"><Icon name="chevron-right" size="s15" /></span></a></li>
);

export const More = memo(function More() {
  const { size, layout } = usePlace();
  const rest = [...new Set(leaves(layout))].slice(4);
  return (
    <div className={listClass(size, "list ")}>
      <div className="top"><h1>More</h1></div>
      <div className="scroll"><ul className="links">
        {rest.map((panel) => <Link key={panel} href={`/${panel}`} icon={PANELS[panel][1]}>{PANELS[panel][0]}</Link>)}
        <Link href="/settings/appearance" icon="settings">Appearance</Link>
        <Link href="/settings/notifications" icon="bell">Notifications</Link></ul>
        <div className="signout"><button type="button" className="btn secondary" onClick={() => signOut(null)}><Icon name="lock" size="s20" />Sign out of this device</button></div></div>
    </div>
  );
});

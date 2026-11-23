// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The bottom toolbar's menus for a narrow column: the label colors as one button, the less used
// filters as another, and the view tools as a third. Each is always in the bar's markup and the
// stylesheet shows it in place of the controls it holds (`@container center`, styles.css), so
// nothing is measured in script and nothing is laid out twice.
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { ListFilter, MoreHorizontal } from "lucide-react";
import { menuKeys, useCardMenu } from "./cardMenu";
import { StarIcon } from "./icons";
import { LABEL_COLORS } from "./library";
import { t } from "./i18n";

const LABEL_NAMES = ["red", "yellow", "green", "blue", "purple"];

function Menu({ className, label, button, on, children }: {
  className: string; label: string; button: ReactNode; on?: boolean; children: (close: () => void) => ReactNode;
}) {
  const { at, close, toggle, box, menu } = useCardMenu();
  return (
    <span className={`tool-menu ${className}`} ref={box}>
      <button type="button" className={`tool${on || at ? " on" : ""}`} aria-haspopup="menu" aria-expanded={!!at}
              title={label} aria-label={label} onClick={(e) => { toggle(e, true); }}>{button}</button>
      {at && createPortal(
        <div ref={menu} className="context-menu module-menu toolbar-menu" role="menu" aria-label={label}
             style={{ position: "fixed", left: at.x, top: at.y }} onKeyDown={menuKeys}>
          {children(close)}
        </div>, document.body)}
    </span>
  );
}

// the five label colors and "no label", each its own switch: the menu stays open to pick several
export function LabelsMenu({ labels, unlabeled, onLabel, onUnlabeled }: {
  labels: number; unlabeled: boolean; onLabel: (k: number) => void; onUnlabeled: () => void;
}) {
  const any = labels !== 0 || unlabeled;
  return (
    <Menu className="collapsed-labels" label={t("library.toolbar.labels")} on={any}
          button={<span className="swatch-stack" aria-hidden><i style={{ background: LABEL_COLORS[0] }} /><i style={{ background: LABEL_COLORS[2] }} /><i style={{ background: LABEL_COLORS[3] }} /></span>}>
      {() => <>
        {LABEL_COLORS.map((c, k) => (
          <button key={c} type="button" role="menuitemcheckbox" aria-checked={!!(labels & (1 << k))} onClick={() => onLabel(k)}>
            <span className="check">{labels & (1 << k) ? "✓" : ""}</span>
            <i className="menu-dot" style={{ background: c }} />
            <span className="name">{t(`library.menu.label-${LABEL_NAMES[k]}`)}</span>
          </button>
        ))}
        <button type="button" role="menuitemcheckbox" aria-checked={unlabeled} onClick={onUnlabeled}>
          <span className="check">{unlabeled ? "✓" : ""}</span>
          <i className="menu-dot" style={{ background: "#9ca3af" }} />
          <span className="name">{t("library.toolbar.withoutLabel")}</span>
        </button>
      </>}
    </Menu>
  );
}

// the reject and edited filters, which cycle through their states: each row says the state a click leaves
export function MoreFiltersMenu({ rejected, edited, rejectedTitle, editedTitle, onRejected, onEdited }: {
  rejected: boolean; edited: boolean; rejectedTitle: string; editedTitle: string; onRejected: () => void; onEdited: () => void;
}) {
  return (
    <Menu className="collapsed-more" label={t("library.toolbar.moreFilters")} on={rejected || edited} button={<ListFilter size={17} strokeWidth={1.8} />}>
      {() => <>
        <button type="button" role="menuitemcheckbox" aria-checked={rejected} onClick={onRejected}>
          <span className="check">{rejected ? "✓" : ""}</span><span className="name">{rejectedTitle}</span>
        </button>
        <button type="button" role="menuitemcheckbox" aria-checked={edited} onClick={onEdited}>
          <span className="check">{edited ? "✓" : ""}</span><span className="name">{editedTitle}</span>
        </button>
      </>}
    </Menu>
  );
}

// the controls a narrow column hides (view tools, pin, Loupe, two panel toggles), as a row of the same buttons
export function OverflowMenu({ render }: { render: () => ReactNode }) {
  // `render` runs while the menu is open, when the controls the column hides can be asked for
  return (
    <Menu className="overflow" label={t("library.toolbar.overflow")} button={<MoreHorizontal size={17} strokeWidth={1.8} />}>
      {() => <div className="overflow-row">{render()}</div>}
    </Menu>
  );
}

// the star filter as one button: "n stars and up", or any rating
export function StarsMenu({ stars, onStars }: { stars: number; onStars: (n: number) => void }) {
  return (
    <Menu className="collapsed-stars" label={t("library.toolbar.rating")} on={stars > 0}
          button={<><StarIcon filled={stars > 0} /><span className="stars-count" aria-hidden>{stars || ""}</span></>}>
      {(close) => <>
        {[0, 1, 2, 3, 4, 5].map((n) => (
          <button key={n} type="button" role="menuitemradio" aria-checked={stars === n} onClick={() => { onStars(n); close(); }}>
            <span className="check">{stars === n ? "✓" : ""}</span>
            <span className="name">{n === 0 ? t("library.toolbar.anyRating") : t("library.toolbar.starsAndUp", { count: n })}</span>
          </button>
        ))}
      </>}
    </Menu>
  );
}

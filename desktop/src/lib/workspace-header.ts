import type { IDockviewGroupPanel, IDockviewPanel, IHeaderActionsRenderer } from "dockview";
import "./workspace-header.css";

type DuplicateMode = "duplicate" | "right" | "below";
interface HeaderActions {
  duplicate: (mode: DuplicateMode, panel: IDockviewPanel) => void;
  open: () => void;
  menuChanged: (close?: () => void) => void;
}

export function createWorkspaceHeader(group: IDockviewGroupPanel, actions: HeaderActions): IHeaderActionsRenderer {
  const element = document.createElement("div");
  element.className = "workspace-header-actions";
  const stop = (event: Event) => event.stopPropagation();
  for (const name of ["pointerdown", "mousedown", "click", "dblclick", "dragstart"]) element.addEventListener(name, stop);
  const iconButton = (label: string, paths: string) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "workspace-header-button";
    button.title = label;
    button.setAttribute("aria-label", label);
    button.innerHTML = `<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
    element.append(button);
    return button;
  };
  const duplicate = (mode: DuplicateMode) => {
    if (group.activePanel) actions.duplicate(mode, group.activePanel);
  };
  const split = iconButton("向右分屏", '<rect x="2" y="2.5" width="12" height="11" rx="1"/><path d="M8 2.5v11"/>');
  split.onclick = () => duplicate("right");
  const more = iconButton("更多操作", '<circle cx="3" cy="8" r=".65"/><circle cx="8" cy="8" r=".65"/><circle cx="13" cy="8" r=".65"/>');
  more.setAttribute("aria-haspopup", "menu");
  more.setAttribute("aria-expanded", "false");
  let closeMenu: (() => void) | undefined;
  const showMenu = () => {
    if (closeMenu) { closeMenu(); return; }
    const menu = document.createElement("div");
    menu.className = "workspace-header-menu";
    menu.id = `workspace-menu-${group.id}`;
    menu.setAttribute("role", "menu");
    menu.setAttribute("aria-label", "标签组操作");
    const items: HTMLButtonElement[] = [];
    const close = () => {
      if (!closeMenu) return;
      closeMenu = undefined;
      menu.remove();
      more.setAttribute("aria-expanded", "false");
      more.removeAttribute("aria-controls");
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("pointercancel", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", key, true);
      actions.menuChanged();
    };
    const outside = (event: PointerEvent) => {
      if (!menu.contains(event.target as Node) && !more.contains(event.target as Node)) close();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" || event.key === "Tab") {
        close();
        more.focus();
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); }
      } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault(); event.stopPropagation();
        const index = items.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
          : (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
        items[next].focus();
      }
    };
    const item = (label: string, action: () => void) => {
      const button = document.createElement("button");
      button.type = "button";
      button.setAttribute("role", "menuitem");
      button.textContent = label;
      button.onclick = () => { close(); action(); };
      menu.append(button);
      items.push(button);
    };
    item("向下分屏", () => duplicate("below"));
    item("新建独立标签", () => duplicate("duplicate"));
    item("打开服务连接", actions.open);
    closeMenu = close;
    actions.menuChanged(close);
    more.setAttribute("aria-expanded", "true");
    more.setAttribute("aria-controls", menu.id);
    document.body.append(menu);
    const anchor = more.getBoundingClientRect();
    const bounds = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(4, Math.min(anchor.right - bounds.width, window.innerWidth - bounds.width - 4))}px`;
    menu.style.top = `${Math.max(4, Math.min(anchor.bottom + 4, window.innerHeight - bounds.height - 4))}px`;
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("pointercancel", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", key, true);
    items[0].focus();
  };
  more.onclick = showMenu;
  more.onkeydown = event => {
    if (event.key === "ArrowDown" && !closeMenu) { event.preventDefault(); showMenu(); }
  };
  return { element, init: () => {}, dispose: () => {
    closeMenu?.();
    element.remove();
  } };
}

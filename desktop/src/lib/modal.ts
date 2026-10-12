/**
 * 让 `<dialog>` 以真正的模态方式打开。
 *
 * 之前所有对话框写的是 `open` 属性（`<dialog open>`），那是非模态：元素不进
 * top layer，于是按 UA 样式的 position:absolute 停在它在文档里的静态位置——
 * 实测落在 y=720，740 高的视口里只露出 20px，等于完全打不开。同理
 * `::backdrop` 不渲染、Esc 关闭无效、背景仍可点击。
 *
 * 改用 showModal()/close() 后，这些都由浏览器原生保证。
 */

/** 把 open 状态同步到 dialog。拆出来是为了在 update 与首次调用时复用。 */
function sync(node: HTMLDialogElement, open: boolean): void {
  if (open) {
    if (!node.open) node.showModal();
    return;
  }
  if (node.open) node.close();
}

/**
 * Svelte action：参数变化时自动重跑 update，所以调用方仍然只管传布尔值。
 *
 * jsdom 早期版本没有 showModal/close。测试环境下降级回 `open` 属性——
 * 那是产品里原本的行为，只是位置不对，不会让测试假装通过。
 */
export function modal(node: HTMLDialogElement, open: boolean): { update(next: boolean): void } {
  if (typeof node.showModal !== "function") {
    node.open = open;
  } else {
    sync(node, open);
  }
  return {
    update(next: boolean) {
      if (typeof node.showModal !== "function") {
        node.open = next;
        return;
      }
      sync(node, next);
    },
  };
}
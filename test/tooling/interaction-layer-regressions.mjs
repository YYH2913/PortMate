import assert from "node:assert/strict";

/** Same-task mutations must not let cached dialog membership leak input. */
export async function checkInteractionLayerRegressions(page) {
  const result = await page.evaluate(async () => {
    const fixture = document.createElement("section");
    const outside = document.createElement("input");
    fixture.append(outside);
    document.body.append(fixture);
    const cases = [];
    let delivered = 0;
    outside.addEventListener("keydown", () => { delivered++; });
    const blocked = (label, expected) => {
      const before = delivered;
      outside.dispatchEvent(new KeyboardEvent("keydown", { key: "q", bubbles: true, cancelable: true }));
      cases.push({ label, expected, actual: before === delivered });
    };
    const layer = (className, zIndex) => {
      const backdrop = document.createElement("div");
      backdrop.className = className;
      backdrop.style.zIndex = String(zIndex);
      const panel = document.createElement("div");
      panel.setAttribute("role", "dialog");
      panel.tabIndex = -1;
      panel.textContent = "Interaction boundary fixture";
      backdrop.append(panel);
      return backdrop;
    };
    const frame = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    let newSibling;
    try {
      blocked("no layer", false);
      const wrapper = document.createElement("div");
      const modal = layer("dialog-backdrop", 10000);
      wrapper.append(modal);
      fixture.append(wrapper);
      blocked("nested insertion before observer delivery", true);
      modal.hidden = true;
      blocked("hidden", false);
      modal.hidden = false;
      modal.setAttribute("aria-hidden", "true");
      blocked("aria-hidden", false);
      modal.removeAttribute("aria-hidden");
      blocked("shown in same task", true);
      modal.className = "not-a-layer";
      blocked("class removed", false);
      modal.className = "dialog-backdrop";
      blocked("class restored", true);
      modal.style.display = "none";
      blocked("display none", false);
      modal.style.display = "";
      modal.style.visibility = "hidden";
      blocked("visibility hidden", false);
      modal.style.visibility = "";
      const other = layer("mcp-approval-backdrop", 10000);
      wrapper.prepend(other);
      blocked("equal-z insertion before existing candidate", true);
      await frame(); // inert/focus restoration is intentionally frame-batched
      cases.push({ label: "equal-z uses DOM order", expected: "active", actual: modal.dataset.modalLayer });
      wrapper.prepend(modal);
      blocked("same-task reorder", true);
      await frame();
      cases.push({ label: "reorder updates active modal", expected: true, actual: other.contains(document.activeElement) });
      modal.style.zIndex = "10001";
      blocked("same-task z-index change", true);
      await frame();
      cases.push({ label: "higher-z wins", expected: true, actual: modal.contains(document.activeElement) });
      const popup = layer("menu-popover", 20000);
      fixture.append(popup);
      blocked("modal has priority over higher-z popup", true);
      await frame();
      cases.push({ label: "popup cannot steal modal focus", expected: true, actual: modal.contains(document.activeElement) });
      newSibling = document.createElement("button");
      document.body.append(newSibling);
      await frame();
      cases.push({ label: "new outside siblings become inert", expected: true, actual: newSibling.inert });
      wrapper.remove();
      await frame();
      blocked("popup remains after modal subtree removed", true);
      popup.remove();
      blocked("all layers removed in same task", false);
      await frame();
      cases.push({ label: "inert restored", expected: false, actual: newSibling.inert || outside.inert });
      // The index must not turn ordinary terminal-like span churn back into
      // whole-document modal queries. Reads happen before observer delivery.
      const query = Document.prototype.querySelectorAll;
      let scans = 0;
      Document.prototype.querySelectorAll = function(selector, ...args) {
        if (selector.includes(".dialog-backdrop") || selector.includes(".menu-popover")) scans++;
        return query.call(this, selector, ...args);
      };
      try {
        const rows = document.createElement("div");
        fixture.append(rows);
        for (let index = 0; index < 100; index++) {
          const span = document.createElement("span");
          span.textContent = `echo ${index}`;
          rows.replaceChildren(span);
          blocked(`typing after echo ${index}`, false);
        }
        await frame();
        cases.push({ label: "no repeated document layer scans", expected: 0, actual: scans });
      } finally { Document.prototype.querySelectorAll = query; }
    } finally {
      newSibling?.remove();
      fixture.remove();
      await frame();
    }
    return cases;
  });
  for (const item of result) assert.equal(item.actual, item.expected, `interaction boundary: ${item.label}; ${JSON.stringify(result.filter(item => item.actual !== item.expected))}`);
  console.log(JSON.stringify({ interactionLayerRegressions: result.length, passed: true }));
}

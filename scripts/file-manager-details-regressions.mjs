import assert from "node:assert/strict";

export async function checkFileManagerDetails(context, appUrl, screenshotPrefix = "/tmp/portmate-file-manager") {
  const page = await context.newPage();
  const errors = [];
  let originalPreference;
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(appUrl);
    await page.locator('.terminal-host[data-terminal-ready="true"]').first().waitFor();
    originalPreference = await page.evaluate(() => localStorage.getItem("portmate.language.v1"));
    const message = key => page.evaluate(async key => (await import("/src/i18n.ts")).t(key), key);
    await page.locator('.menu-trigger[aria-controls="menu-workspace"]').click();
    await page.locator("#menu-workspace").getByRole("button", { name: await message("file-manager"), exact: true }).click();
    const local = page.locator('[data-file-pane="local"]');
    const remote = page.locator('[data-file-pane="remote"]');
    await local.getByRole("button", { name: await message("new-file"), exact: true }).waitFor();

    const entries = [
      { name: "file10.txt", path: "/details/file10.txt", isDir: false, size: 512, modified: "2026-09-16T02:00:00Z" },
      { name: "folder2", path: "/details/folder2", isDir: true, size: 4096, modified: null },
      { name: "file2.log", path: "/details/file2.log", isDir: false, size: 2048, modified: "2026-09-16T09:00:00+08:00" },
      { name: "folder1", path: "/details/folder1", isDir: true, size: 0, modified: null },
      { name: "file1.txt", path: "/details/file1.txt", isDir: false, size: 0, modified: "invalid-time" },
      { name: " spaced العربية.txt ", path: "/details/ spaced العربية.txt ", isDir: false, size: 2, modified: null },
    ];
    async function load(pane, path, data) {
      await page.evaluate(() => { window.__deferFileLoads = true; });
      await pane.getByRole("textbox").fill(path);
      await pane.getByRole("textbox").press("Enter");
      await page.waitForFunction(path => window.__pendingFileLoads.some(item => item.args.request.path === path), path);
      await page.evaluate(({ path, data }) => {
        const index = window.__pendingFileLoads.findIndex(item => item.args.request.path === path);
        window.__pendingFileLoads.splice(index, 1)[0].resolve(data);
        window.__deferFileLoads = false;
      }, { path, data });
      await page.waitForFunction(({ path, count }) => {
        const pane = [...document.querySelectorAll("[data-file-pane]")].find(pane => pane.dataset.fileDirectory === path);
        return pane?.getAttribute("aria-busy") === "false" && pane.querySelectorAll("[data-file-path]").length === count;
      }, { path, count: data.length });
    }
    const order = pane => pane.locator(".file-entry-name").allTextContents();
    const row = (pane, name) => pane.locator("[data-file-path]").filter({ has: page.locator(".file-entry-name", { hasText: new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`) }) });
    await load(local, "/details", entries);
    await load(remote, "/remote-details", entries.map(entry => ({ ...entry, path: entry.path.replace("/details", "/remote-details") })));
    const ascendingNames = ["folder1", "folder2", " spaced العربية.txt ", "file1.txt", "file2.log", "file10.txt"];
    assert.deepEqual(await order(local), ascendingNames, "file names were not naturally sorted with directories first");
    assert.equal(await row(local, "file2.log").locator(".file-entry-size").textContent(), "2.0 KiB");
    assert.equal(await row(local, "file1.txt").locator(".file-entry-modified").textContent(), await message("unknown"));
    assert.equal(await row(local, "file10.txt").locator(".file-entry-modified").getAttribute("title"), entries[0].modified);
    assert.equal(await row(local, "folder2").locator(".file-entry-size").textContent(), "—", "directory inode size was shown as its content size");

    await local.locator('[data-file-sort="size"]').click();
    assert.deepEqual(await order(local), ["folder1", "folder2", "file1.txt", " spaced العربية.txt ", "file10.txt", "file2.log"]);
    assert.deepEqual(await order(remote), ascendingNames, "sorting the local pane also reordered the remote pane");
    await local.locator('[data-file-sort="size"]').click();
    assert.deepEqual(await order(local), ["folder1", "folder2", "file2.log", "file10.txt", " spaced العربية.txt ", "file1.txt"]);
    await local.locator('[data-file-sort="modified"]').click();
    assert.deepEqual((await order(local)).slice(2), ["file2.log", "file10.txt", " spaced العربية.txt ", "file1.txt"]);
    await local.locator('[data-file-sort="modified"]').click();
    assert.deepEqual((await order(local)).slice(2), ["file10.txt", "file2.log", " spaced العربية.txt ", "file1.txt"]);
    await local.locator('[data-file-sort="name"]').click();
    await row(local, "file1.txt").click();
    await row(local, "file10.txt").click({ modifiers: ["Shift"] });
    assert.deepEqual(await local.locator('[data-file-path][aria-selected="true"] .file-entry-name').allTextContents(), ["file1.txt", "file2.log", "file10.txt"], "Shift selected a range in the unsorted backend order");
    assert.match(await local.locator(".file-list-summary").innerText(), /2\.5 KiB/, "selection summary includes directory metadata or omits file bytes");

    // Viewing metadata must not rename, normalize, or translate a selected path.
    await page.evaluate(() => { window.__deferFileProperties = true; });
    await row(local, " spaced العربية.txt ").dblclick();
    await page.waitForFunction(() => window.__pendingFileProperties.length > 0);
    const requestedPath = await page.evaluate(() => window.__pendingFileProperties.at(-1).args.request.path);
    assert.equal(requestedPath, "/details/ spaced العربية.txt ");
    await page.evaluate(() => window.__pendingFileProperties.pop().resolve({
      name: " spaced العربية.txt ", path: "/details/ spaced العربية.txt ", remote: false,
      kind: "file", isFile: true, isDir: false, isSymlink: false, size: 2048,
      permissions: 0o100644, modified: "2026-09-16T02:00:00Z", accessed: null, created: null,
    }));
    const properties = page.locator(".file-properties-dialog");
    await properties.locator(".property-grid").waitFor();
    assert.match(await properties.innerText(), /0644 \(rw-r--r--\)/);
    assert.equal(await properties.locator(".file-property-raw").nth(1).textContent(), requestedPath);
    await properties.locator(".utility-actions button").click();

    const pathBefore = await row(local, " spaced العربية.txt ").getAttribute("data-file-path");
    for (const locale of ["ar", "fr", "ru", "es", "en", "zh"]) {
      await page.evaluate(async locale => (await import("/src/i18n.ts")).setLanguagePreference(locale), locale);
      assert.equal(await local.locator('[data-file-sort="name"]').getAttribute("data-sort-direction"), "asc");
      assert.deepEqual(await order(local), ascendingNames, "changing language changed file order");
      assert.equal(await row(local, " spaced العربية.txt ").getAttribute("data-file-path"), pathBefore);
      assert.equal(await local.getByRole("textbox").evaluate(element => getComputedStyle(element).direction), "ltr");
      assert.equal(await row(local, " spaced العربية.txt ").locator(".file-entry-name").evaluate(element => getComputedStyle(element).direction), "ltr");
      assert.equal(await row(local, "file2.log").locator(".file-entry-size").evaluate(element => getComputedStyle(element).direction), "ltr", "RTL reversed the byte quantity and unit");
      assert(!/file-(?:kind|sort|listed|selected|empty|extension)/.test(await local.innerText()), "untranslated file manager message identifiers leaked");
    }
    await page.screenshot({ path: `${screenshotPrefix}-details.png`, fullPage: true });
    await page.evaluate(async () => (await import("/src/i18n.ts")).setLanguagePreference("ar"));
    // The desktop shell has an explicit 1024px minimum; exercise a narrow file pane
    // within the supported shell width instead of attributing that global limit to the list.
    await page.setViewportSize({ width: 1100, height: 760 });
    const geometry = await local.evaluate(pane => {
      const list = pane.querySelector(".file-details-list");
      const rect = element => { const r = element.getBoundingClientRect(); return { left: r.left, right: r.right }; };
      return { paneWidth: pane.clientWidth, listWidth: list.clientWidth, scrollWidth: list.scrollWidth, documentWidth: document.documentElement.scrollWidth, viewportWidth: innerWidth,
        listRect: rect(list), checkboxRect: rect(list.querySelector('[data-file-path] input')),
      };
    });
    assert(geometry.scrollWidth >= geometry.listWidth && geometry.listWidth <= geometry.paneWidth + 1 && geometry.documentWidth <= geometry.viewportWidth + 1,
      `file details escaped the pane instead of scrolling within it: ${JSON.stringify(geometry)}`);
    assert(geometry.checkboxRect.left >= geometry.listRect.left && geometry.checkboxRect.right <= geometry.listRect.right,
      `switching into RTL left the selection column clipped: ${JSON.stringify(geometry)}`);
    await page.screenshot({ path: `${screenshotPrefix}-arabic-details.png`, fullPage: true });
    // Parent navigation must keep full Windows roots and POSIX filename bytes.
    for (const [pane, currentPath, expectedParent] of [
      [local, "F:\\folder", "F:\\"],
      [local, "\\\\?\\F:\\folder", "\\\\?\\F:\\"],
      [local, "\\\\server\\share\\folder", "\\\\server\\share\\"],
      [local, "\\\\server\\share\\", "\\\\server\\share\\"],
      [remote, "/remote/part\\name", "/remote"],
    ]) {
      await load(pane, currentPath, []);
      const beforeCalls = await page.evaluate(() => window.__invokeCalls.length);
      await pane.getByRole("button", { name: await message("file-parent-directory"), exact: true }).click();
      await page.waitForFunction(start => window.__invokeCalls.slice(start).some(call => call.command === "list_files"), beforeCalls);
      const requested = await page.evaluate(start => window.__invokeCalls.slice(start).find(call => call.command === "list_files").args.request.path, beforeCalls);
      assert.equal(requested, expectedParent, "parent navigation produced a bare drive or escaped the share root");
      await page.waitForFunction(path => [...document.querySelectorAll("[data-file-directory]")].some(pane => pane.dataset.fileDirectory === path && pane.getAttribute("aria-busy") === "false"), expectedParent);
    }
    await load(remote, ".", []);
    for (const expectedParent of ["..", "../.."]) {
      await remote.getByRole("button", { name: await message("file-parent-directory"), exact: true }).click();
      await page.waitForFunction(path => document.querySelector('[data-file-pane="remote"]')?.dataset.fileDirectory === path, expectedParent);
      assert.equal(await remote.getByRole("textbox").inputValue(), expectedParent, "relative navigation did not continue to the next parent");
    }
    assert.deepEqual(errors, [], "file manager details produced browser errors");
  } finally {
    await page.evaluate(async preference => {
      const { setLanguagePreference } = await import("/src/i18n.ts");
      setLanguagePreference(preference ?? "system");
      if (preference === null) localStorage.removeItem("portmate.language.v1");
    }, originalPreference).catch(() => {});
    await page.close();
  }
}

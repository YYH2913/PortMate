import { describe, expect, it } from "vitest";
import {
  MAX_FILE_NAVIGATION_HISTORY,
  createFileNavigationHistory,
  currentFileNavigationPath,
  fileNavigationTarget,
  recordFileNavigation,
  restoreFileNavigation,
} from "./file-navigation-state";

describe("file navigation history", () => {
  it("records successful navigation and drops the forward branch", () => {
    let history = createFileNavigationHistory("/");
    history = recordFileNavigation(history, "/etc");
    history = recordFileNavigation(history, "/var");
    history = restoreFileNavigation(history, 1);
    history = recordFileNavigation(history, "/tmp");

    expect(history).toEqual({ paths: ["/", "/etc", "/tmp"], index: 2 });
    expect(currentFileNavigationPath(history)).toBe("/tmp");
    expect(fileNavigationTarget(history, -1)).toEqual({ path: "/etc", index: 1 });
    expect(fileNavigationTarget(history, 1)).toBeNull();
  });

  it("keeps bounded history and rejects invalid restore targets", () => {
    let history = createFileNavigationHistory("/0");
    for (let index = 1; index <= MAX_FILE_NAVIGATION_HISTORY + 2; index += 1) {
      history = recordFileNavigation(history, `/${index}`);
    }

    expect(history.paths).toHaveLength(MAX_FILE_NAVIGATION_HISTORY);
    expect(history.paths[0]).toBe("/3");
    expect(currentFileNavigationPath(history)).toBe(`/${MAX_FILE_NAVIGATION_HISTORY + 2}`);
    expect(restoreFileNavigation(history, -1)).toBe(history);
    expect(fileNavigationTarget(history, 1.5)).toBeNull();
  });
});

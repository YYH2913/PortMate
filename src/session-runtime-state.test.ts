import { describe, expect, it } from "vitest";
import { sessionConnectionAction } from "./session-runtime-state";

describe("session runtime state", () => {
  it("allows active and pending transports to be cancelled", () => {
    expect(sessionConnectionAction("connected")).toBe("disconnect");
    expect(sessionConnectionAction("connecting")).toBe("disconnect");
    expect(sessionConnectionAction("reconnecting")).toBe("disconnect");
  });

  it("offers connection for inactive terminal states", () => {
    expect(sessionConnectionAction("disconnected")).toBe("connect");
    expect(sessionConnectionAction("blocked")).toBe("connect");
    expect(sessionConnectionAction("error")).toBe("connect");
  });
});

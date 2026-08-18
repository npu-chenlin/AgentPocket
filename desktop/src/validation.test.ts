import { describe, expect, it } from "vitest";
import { validateServerDraft } from "./validation";

describe("validateServerDraft", () => {
  it("accepts a valid dsh server", () => {
    expect(
      validateServerDraft({
        name: "Work",
        host: "100.64.0.2",
        port: 3080,
        token: "",
        backend: "dsh",
      }),
    ).toEqual({});
  });

  it.each(["http://host", "host:3080", "host/path", "bad host"])(
    "rejects host %s",
    (host) => {
      expect(
        validateServerDraft({
          name: "Work",
          host,
          port: 3080,
          token: "",
          backend: "dsh",
        }).host,
      ).toBeTruthy();
    },
  );

  it.each([0, 65_536, Number.NaN, 1.5])("rejects port %s", (port) => {
    expect(
      validateServerDraft({
        name: "Work",
        host: "host",
        port,
        token: "",
        backend: "dsh",
      }).port,
    ).toBeTruthy();
  });

  it("requires a visible name", () => {
    expect(
      validateServerDraft({
        name: "   ",
        host: "host",
        port: 3080,
        token: "",
        backend: "kimi",
      }).name,
    ).toBeTruthy();
  });
});

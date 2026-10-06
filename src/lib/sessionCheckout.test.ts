import { describe, expect, it } from "vitest";

import { samePath, sessionCheckout, sessionFolderName } from "./sessionCheckout";

const ROOT = "/home/u/work/central";

describe("sessionCheckout", () => {
  it("is null for no session and for a session in the project root", () => {
    expect(sessionCheckout(undefined, ROOT)).toBeNull();
    expect(sessionCheckout({ cwd: ROOT }, ROOT)).toBeNull();
    expect(sessionCheckout({ cwd: `${ROOT}/` }, ROOT)).toBeNull();
  });

  it("follows the worktree first, then the session's own folder", () => {
    expect(sessionCheckout({ cwd: "/wt/a", worktree: { path: "/wt/a" } }, ROOT)).toBe("/wt/a");
    expect(sessionCheckout({ cwd: "/home/u/work/backend" }, ROOT)).toBe("/home/u/work/backend");
  });
});

describe("sessionFolderName", () => {
  it("names only sessions in another folder, never worktree or root ones", () => {
    expect(sessionFolderName({ cwd: ROOT }, ROOT)).toBeNull();
    expect(sessionFolderName({ cwd: "/wt/a", worktree: { path: "/wt/a" } }, ROOT)).toBeNull();
    expect(sessionFolderName({ cwd: "/home/u/work/backend/" }, ROOT)).toBe("backend");
    expect(sessionFolderName({ cwd: "C:\\work\\backend" }, "C:\\work\\central")).toBe("backend");
  });
});

describe("samePath", () => {
  it("ignores trailing separators only", () => {
    expect(samePath("/a/b/", "/a/b")).toBe(true);
    expect(samePath("/a/b", "/a/c")).toBe(false);
    expect(samePath("/", "/")).toBe(true);
  });
});

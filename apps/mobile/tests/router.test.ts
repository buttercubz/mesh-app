// Router, back handling and invite deep links (WP-18, DESIGN §11.1, §11.3).
import { beforeEach, describe, expect, it } from "vitest";

import { handleIncomingUrl, isInviteUrl } from "../src/app/deepLinks";
import {
  INITIAL_SCREEN,
  SCREEN,
  currentRoute,
  getScreen,
  handleBackPress,
  pushBackHandler,
  registerScreen,
  registeredScreenIds,
  unregisterScreen,
  useRouter,
} from "../src/app/router";

const Dummy = () => null;

beforeEach(() => {
  useRouter.getState().reset();
  for (const id of registeredScreenIds()) unregisterScreen(id);
});

describe("screen registry", () => {
  it("registers screens and bumps the registry version", () => {
    const before = useRouter.getState().registryVersion;
    registerScreen("lobby", Dummy);
    expect(getScreen("lobby")?.component).toBe(Dummy);
    expect(useRouter.getState().registryVersion).toBe(before + 1);
    expect(() => registerScreen("Bad Id", Dummy)).toThrow(/invalid screen id/);
  });

  it("pushes, replaces, goes back and resets", () => {
    const r = useRouter.getState();
    expect(currentRoute().id).toBe(INITIAL_SCREEN);
    r.push("lobby", { groupId: "g" });
    r.push("ride");
    expect(currentRoute()).toMatchObject({ id: "ride", params: {} });
    useRouter.getState().replace("members");
    expect(useRouter.getState().stack.map((s) => s.id)).toEqual(["home", "lobby", "members"]);
    expect(useRouter.getState().back()).toBe(true);
    expect(currentRoute()).toMatchObject({ id: "lobby", params: { groupId: "g" } });
    useRouter.getState().reset("ride");
    expect(useRouter.getState().back()).toBe(false);
  });

  it("gives every push a fresh key (a re-pushed screen remounts)", () => {
    useRouter.getState().push("lobby");
    const k1 = currentRoute().key;
    useRouter.getState().back();
    useRouter.getState().push("lobby");
    expect(currentRoute().key).not.toBe(k1);
  });
});

describe("back press", () => {
  it("asks back handlers first (newest first), then the stack", () => {
    useRouter.getState().push("lobby");
    const order: string[] = [];
    const removeA = pushBackHandler(() => (order.push("a"), false));
    const removeB = pushBackHandler(() => (order.push("b"), true));
    expect(handleBackPress()).toBe(true);
    expect(order).toEqual(["b"]);
    removeB();
    expect(handleBackPress()).toBe(true);
    expect(order).toEqual(["b", "a"]);
    expect(currentRoute().id).toBe("home");
    removeA();
    expect(handleBackPress()).toBe(false);
  });

  it("never pops a screen that blocks back (the ride screen)", () => {
    registerScreen(SCREEN.ride, Dummy, { blockBack: true });
    useRouter.getState().push(SCREEN.ride);
    expect(handleBackPress()).toBe(true);
    expect(currentRoute().id).toBe(SCREEN.ride);
  });
});

describe("invite deep links", () => {
  it("accepts only convlinkvoice://join links", () => {
    expect(isInviteUrl("convlinkvoice://join?v=1#abc")).toBe(true);
    expect(isInviteUrl("CONVLINKVOICE://join#x")).toBe(true);
    expect(isInviteUrl("convlinkvoice://joinx")).toBe(false);
    expect(isInviteUrl("https://example.com/join")).toBe(false);
    expect(isInviteUrl("convlinkvoice://settings")).toBe(false);
    expect(isInviteUrl(42)).toBe(false);
    expect(isInviteUrl("convlinkvoice://join#" + "a".repeat(5000))).toBe(false);
  });

  it("opens the join screen with the URL as an in-memory param", () => {
    expect(handleIncomingUrl("https://evil.example/join")).toBe(false);
    expect(currentRoute().id).toBe("home");
    expect(handleIncomingUrl(" convlinkvoice://join?v=1#abc ")).toBe(true);
    expect(currentRoute()).toMatchObject({ id: SCREEN.join, params: { url: "convlinkvoice://join?v=1#abc" } });
  });
});

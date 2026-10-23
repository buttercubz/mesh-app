// Vitest setup (jsdom): React Testing Library cleanup and the pointer APIs jsdom lacks.
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  cleanup();
});

// jsdom ships PointerEvent but not pointer capture; the button must work without it
// (it guards the call), but tests spy on it, so provide inert implementations.
// Files that opt into the node environment (no DOM) skip this.
if (typeof HTMLElement !== "undefined") {
  const proto = HTMLElement.prototype as HTMLElement & {
    setPointerCapture?: (id: number) => void;
    releasePointerCapture?: (id: number) => void;
    hasPointerCapture?: (id: number) => boolean;
  };
  proto.setPointerCapture ??= function setPointerCapture() {};
  proto.releasePointerCapture ??= function releasePointerCapture() {};
  proto.hasPointerCapture ??= function hasPointerCapture() {
    return false;
  };
}

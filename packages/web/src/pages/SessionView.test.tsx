// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act, cleanup } from "@testing-library/react";
import { SessionView } from "./SessionView.js";

// Share state with the hoisted mock: capture the opts passed to connectSession
// so the test can drive connection state without a real WebRTC stack.
const h = vi.hoisted(() => ({
  opts: null as null | { onState: (s: string) => void },
  session: { close: vi.fn(), sendInput: vi.fn(), getStats: vi.fn().mockResolvedValue(null) },
}));

vi.mock("../rtc.js", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../rtc.js")>();
  return {
    ...actual,
    connectSession: (_b: string, _t: string, _d: string, opts: { onState: (s: string) => void }) => {
      h.opts = opts;
      return h.session;
    },
  };
});

const device = { id: "dev-1", name: "My Mac", online: true };

beforeEach(() => {
  cleanup();
  h.opts = null;
  vi.clearAllMocks();
  Object.defineProperty(document, "fullscreenElement", { value: null, configurable: true, writable: true });
  (HTMLElement.prototype as unknown as { requestFullscreen: () => Promise<void> }).requestFullscreen = vi
    .fn()
    .mockResolvedValue(undefined);
});

describe("SessionView fullscreen", () => {
  it("enables the button only when connected and requests fullscreen on click", () => {
    render(<SessionView token="t" device={device} onExit={() => {}} />);
    const btn = screen.getByTestId("fullscreen-btn") as HTMLButtonElement;

    expect(btn.disabled).toBe(true); // not connected yet
    act(() => h.opts!.onState("connected"));
    expect(btn.disabled).toBe(false);

    fireEvent.click(btn);
    const video = screen.getByTestId("remote-surface");
    expect(video.requestFullscreen).toHaveBeenCalledTimes(1);
  });

  it("fills the browser viewport (CSS overlay) without the Fullscreen API", () => {
    render(<SessionView token="t" device={device} onExit={() => {}} />);
    act(() => h.opts!.onState("connected"));
    const btn = screen.getByTestId("maximize-btn") as HTMLButtonElement;
    const video = screen.getByTestId("remote-surface") as HTMLVideoElement;

    expect(btn.disabled).toBe(false);
    expect(screen.queryByTestId("maximize-exit")).toBeNull();
    expect(video.style.position).not.toBe("fixed");

    fireEvent.click(btn); // enter fill-window
    expect(video.style.position).toBe("fixed"); // overlay, not OS fullscreen
    expect(video.requestFullscreen).not.toHaveBeenCalled();
    expect(screen.getByTestId("maximize-exit")).toBeTruthy();

    fireEvent.click(screen.getByTestId("maximize-exit")); // exit
    expect(video.style.position).not.toBe("fixed");
    expect(screen.queryByTestId("maximize-exit")).toBeNull();
  });

  it("reflects fullscreen state in the button label", () => {
    render(<SessionView token="t" device={device} onExit={() => {}} />);
    act(() => h.opts!.onState("connected"));
    const btn = screen.getByTestId("fullscreen-btn");
    expect(btn.textContent).toContain("Fullscreen");

    // Simulate the browser entering fullscreen on the surface.
    const video = screen.getByTestId("remote-surface");
    Object.defineProperty(document, "fullscreenElement", { value: video, configurable: true, writable: true });
    act(() => document.dispatchEvent(new Event("fullscreenchange")));
    expect(btn.textContent).toContain("Exit fullscreen");
  });

  it("sends a chord as kdown…kup in reverse on combo click", () => {
    render(<SessionView token="t" device={device} onExit={() => {}} />);
    expect((screen.getByTestId("combo-Copy") as HTMLButtonElement).disabled).toBe(true);
    act(() => h.opts!.onState("connected"));
    fireEvent.click(screen.getByTestId("combo-Copy"));
    const sent = h.session.sendInput.mock.calls.map((c) => c[0]);
    expect(sent).toEqual([
      { t: "kdown", code: "MetaLeft" },
      { t: "kdown", code: "KeyC" },
      { t: "kup", code: "KeyC" },
      { t: "kup", code: "MetaLeft" },
    ]);
  });

  it("toggles the stats HUD", () => {
    render(<SessionView token="t" device={device} onExit={() => {}} />);
    act(() => h.opts!.onState("connected"));
    expect(screen.queryByTestId("stats-hud")).toBeNull();
    fireEvent.click(screen.getByTestId("stats-btn"));
    expect(screen.getByTestId("stats-hud")).toBeTruthy();
    fireEvent.click(screen.getByTestId("stats-btn"));
    expect(screen.queryByTestId("stats-hud")).toBeNull();
  });
});

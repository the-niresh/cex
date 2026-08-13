import { describe, expect, it } from "vitest";
import { cn } from "./utils";

/**
 * `cn` is tailwind-merge, and tailwind-merge only knows Tailwind's *stock*
 * scales unless it is told about the theme. This screen's type scale is three
 * names of its own — data, label, micro — and every one of them looks to an
 * unconfigured merge like a *colour*, because `text-` is both the font-size
 * prefix and the text-colour prefix.
 *
 * The consequence is silent and it already bit: any `cn("text-label",
 * "text-ink-4")` dropped the size and left the element inheriting whatever the
 * body was set to. Nothing errors, nothing warns, the label is just the wrong
 * size. These tests pin the merge to the theme so it cannot happen again.
 */
describe("cn", () => {
  it("keeps a theme font size alongside a text colour", () => {
    expect(cn("text-micro", "text-ink-4")).toBe("text-micro text-ink-4");
    expect(cn("text-label", "text-ink-4")).toBe("text-label text-ink-4");
    expect(cn("text-data", "text-buy")).toBe("text-data text-buy");
  });

  it("lets a theme font size override a stock one", () => {
    expect(cn("text-sm", "text-label")).toBe("text-label");
    expect(cn("text-label", "text-sm")).toBe("text-sm");
  });

  it("treats the theme's radii as one group", () => {
    expect(cn("rounded-panel", "rounded-control")).toBe("rounded-control");
    expect(cn("rounded-control", "rounded-pill")).toBe("rounded-pill");
  });

  it("knows the field size, the largest step", () => {
    expect(cn("text-field", "text-ink")).toBe("text-field text-ink");
  });

  it("still merges stock utilities the ordinary way", () => {
    expect(cn("px-2", "px-2.5")).toBe("px-2.5");
    expect(cn("text-ink", "text-ink-2")).toBe("text-ink-2");
  });
});

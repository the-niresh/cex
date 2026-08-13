import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

/**
 * ⚠️ tailwind-merge has to be told about this project's theme, or it silently
 * drops classes.
 *
 * It decides which utilities conflict by matching class *names* against
 * Tailwind's stock scales. `text-` is both the font-size prefix and the
 * text-colour prefix, so an unrecognised `text-label` is read as a colour —
 * and then a real colour after it in the same call wins and the size vanishes.
 * No error, no warning; the element just inherits the body's size.
 *
 * Naming the theme's own scales here puts them back in the group they belong
 * to. See `utils.test.ts`, which pins the behaviour.
 */
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      text: ["data", "label", "micro", "field"],
      radius: ["panel", "control", "pill"],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

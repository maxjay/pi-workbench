// The colour of a thing, by what it is. Every one has the same lightness
// and strength, as theme.css's own colours do in each theme, and only the hue differs: a pull request has a hue of its own
// from its number, and a ticket has its type's, as Azure DevOps colours it.
// The same thing has the same colour wherever its number is written.

/** A colour for a kind of thing. */
export const hue = (degrees: number) => `light-dark(oklch(0.7 0.17 ${degrees}), oklch(0.8 0.15 ${degrees}))`;

/** The hues that mean nothing else here. Not red, orange, yellow, green or the blue of threads: those say something. */
const FREE = [330, 280, 210, 170, 300, 190];

/**
 * A pull request's own colour. Its number is its own in the whole organization, so that is what
 * picks it, and pull requests opened one after another get different ones.
 */
export const pullHue = (pr: number) => hue(FREE[pr % FREE.length]);

/** The hue of a colour written as six hex digits, on the wheel the app's colours are on. */
function hexHue(hex: string) {
  const digits = hex.replace('#', '').slice(-6);
  if (!/^[0-9a-f]{6}$/i.test(digits)) return undefined;
  const [r, g, b] = [0, 2, 4]
    .map((at) => Number.parseInt(digits.slice(at, at + 2), 16) / 255)
    .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const yellow = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return Math.round(((Math.atan2(yellow, a) * 180) / Math.PI + 360) % 360);
}

/**
 * The colour of a kind of ticket: the hue Azure DevOps gives the type when
 * it is known, and else the hue it gives the types every project has.
 */
export function ticketHue(type: string | null | undefined, azure?: string | null) {
  const known = azure ? hexHue(azure) : undefined;
  if (known !== undefined) return hue(known);
  const kind = (type ?? '').toLowerCase();
  if (kind.includes('bug') || kind.includes('issue') || kind.includes('impediment')) return hue(25);
  if (kind.includes('epic')) return hue(45);
  if (kind.includes('feature')) return hue(300);
  if (kind.includes('task')) return hue(95);
  return hue(230);
}

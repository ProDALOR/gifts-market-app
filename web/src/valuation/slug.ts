/** A deep-link gift is a slug, never a free-form URL or a fabricated listing. */
export function normalizeSlug(
  value: string | null | undefined,
): string | undefined {
  if (!value || value.length > 512) return;
  const match = value.match(/^([a-zA-Z0-9]{1,128})-([0-9]+)$/);
  if (!match || +match[2] < 1 || +match[2] > 0xffffffff) return;
  return `${match[1].toLowerCase()}-${+match[2]}`;
}
export const sameSlug = (a: string | null | undefined, b: string) =>
  !!normalizeSlug(a) && normalizeSlug(a) === normalizeSlug(b);

export function duration(s: number | null | undefined): string {
  if (s == null) return '';
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const sec = s % 60;
  const two = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${two(m)}:${two(sec)}` : `${m}:${two(sec)}`;
}

export function count(n: number | null | undefined): string {
  if (n == null) return '';
  const trim = (v: number, unit: string) =>
    (v >= 100 ? v.toFixed(0) : v.toFixed(1).replace(/\.0$/, '')) + unit;
  if (n < 1_000) return String(n);
  if (n < 1_000_000) return trim(n / 1e3, 'K');
  if (n < 1_000_000_000) return trim(n / 1e6, 'M');
  return trim(n / 1e9, 'B');
}

/** "Channel · 1.2M views · 3 days ago", leaving out what is unknown. */
export function meta(parts: Array<string | null | undefined | false>): string {
  return parts.filter(Boolean).join(' · ');
}

/** Thumbnails sometimes come without a scheme. */
export function img(url: string | null | undefined): string | undefined {
  if (!url) return undefined;
  return url.startsWith('//') ? `https:${url}` : url;
}

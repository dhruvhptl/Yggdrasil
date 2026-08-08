export type CitationToken =
  | { kind: 'text'; text: string }
  | { kind: 'cite'; index: number };

/** Split answer text into plain-text and [n] citation tokens. Pure + total. */
export function tokenizeCitations(text: string): CitationToken[] {
  const out: CitationToken[] = [];
  const re = /\[(\d+)\]/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push({ kind: 'text', text: text.slice(last, m.index) });
    out.push({ kind: 'cite', index: parseInt(m[1], 10) });
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ kind: 'text', text: text.slice(last) });
  return out;
}

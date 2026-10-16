// Minimal Markdown subset for GitHub release notes. It produces a plain data
// tree that React renders as elements, so release text can never inject HTML.

export type Inline =
  | { type: 'text'; text: string }
  | { type: 'strong'; children: Inline[] }
  | { type: 'em'; children: Inline[] }
  | { type: 'code'; text: string }
  | { type: 'link'; href: string; children: Inline[] };

export type Block =
  | { type: 'heading'; level: 1 | 2 | 3; children: Inline[] }
  | { type: 'paragraph'; children: Inline[] }
  | { type: 'list'; ordered: boolean; items: Inline[][] }
  | { type: 'rule' };

const inlinePattern = /(`[^`\n]+`)|(\*\*[^*\n]+?\*\*|__[^_\n]+?__)|(\[[^\]\n]+\]\([^)\s]+\))|(\*[^*\s][^*\n]*?\*|_[^_\s][^_\n]*?_)/;

export function parseInline(source: string): Inline[] {
  const result: Inline[] = [];
  let rest = source;
  while (rest) {
    const match = inlinePattern.exec(rest);
    if (!match) {
      result.push({ type: 'text', text: rest });
      break;
    }
    if (match.index > 0) result.push({ type: 'text', text: rest.slice(0, match.index) });
    const token = match[0];
    if (match[1]) result.push({ type: 'code', text: token.slice(1, -1) });
    else if (match[2]) result.push({ type: 'strong', children: parseInline(token.slice(2, -2)) });
    else if (match[3]) {
      const split = token.indexOf('](');
      result.push({ type: 'link', href: token.slice(split + 2, -1), children: parseInline(token.slice(1, split)) });
    } else result.push({ type: 'em', children: parseInline(token.slice(1, -1)) });
    rest = rest.slice(match.index + token.length);
  }
  return result;
}

export function parseMarkdown(source: string): Block[] {
  const blocks: Block[] = [];
  let paragraph: string[] = [];
  let list: { ordered: boolean; items: string[] } | null = null;

  const flushParagraph = () => {
    if (paragraph.length) blocks.push({ type: 'paragraph', children: parseInline(paragraph.join(' ')) });
    paragraph = [];
  };
  const flushList = () => {
    if (list) blocks.push({ type: 'list', ordered: list.ordered, items: list.items.map(parseInline) });
    list = null;
  };

  for (const rawLine of source.replace(/\r\n/g, '\n').split('\n')) {
    const line = rawLine.trim();
    if (!line) {
      flushParagraph();
      flushList();
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flushParagraph();
      flushList();
      const level = Math.min(3, heading[1].length) as 1 | 2 | 3;
      blocks.push({ type: 'heading', level, children: parseInline(heading[2].replace(/\s+#+$/, '')) });
      continue;
    }
    if (/^([-*_])(\s*\1){2,}$/.test(line)) {
      flushParagraph();
      flushList();
      blocks.push({ type: 'rule' });
      continue;
    }
    const bullet = /^[-*+]\s+(.*)$/.exec(line);
    const numbered = /^\d+[.)]\s+(.*)$/.exec(line);
    if (bullet || numbered) {
      flushParagraph();
      const ordered = Boolean(numbered);
      if (list && list.ordered !== ordered) flushList();
      if (!list) list = { ordered, items: [] };
      list.items.push((bullet ?? numbered)![1]);
      continue;
    }
    if (list && /^\s{2,}/.test(rawLine)) {
      // Continuation line of the previous list item.
      list.items[list.items.length - 1] += ` ${line}`;
      continue;
    }
    flushList();
    paragraph.push(line);
  }
  flushParagraph();
  flushList();
  return blocks;
}

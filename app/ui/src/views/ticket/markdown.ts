// A ticket's text is kept by Azure DevOps as HTML. To change it you write
// Markdown. This turns the HTML into the Markdown that makes it again:
// paragraphs, bold and italic, links, lists, headings, code, quotes and
// pictures. Anything else, a table say, is left as the HTML it is, which
// Markdown lets through unchanged. So nothing is lost by changing a part.

const BLOCKS = new Set([
  'P',
  'DIV',
  'UL',
  'OL',
  'LI',
  'PRE',
  'BLOCKQUOTE',
  'H1',
  'H2',
  'H3',
  'H4',
  'H5',
  'H6',
]);

/** What is inside an element, as Markdown that sits in a line. */
function inline(node: Node): string {
  // What Markdown would read as a mark is written so it stays the letter it is: `Vec<Cart>`, `a * b`.
  if (node.nodeType === Node.TEXT_NODE) {
    return (node.textContent ?? '')
      .replace(/\s+/g, ' ')
      .replace(/[\\<*_`]/g, '\\$&')
      .replace(/&(?=#?\w+;)/g, '\\&');
  }
  if (!(node instanceof HTMLElement)) return '';
  const inside = () => [...node.childNodes].map(inline).join('');
  const wrapped = (mark: string) => {
    const words = inside();
    return words.trim() ? `${mark}${words.trim()}${mark}${words.endsWith(' ') ? ' ' : ''}` : words;
  };
  switch (node.tagName) {
    case 'BR':
      return '\n';
    case 'B':
    case 'STRONG':
      return wrapped('**');
    case 'I':
    case 'EM':
      return wrapped('*');
    case 'S':
    case 'DEL':
    case 'STRIKE':
      return wrapped('~~');
    case 'CODE':
      return `\`${node.textContent ?? ''}\``;
    case 'A': {
      const [words, href] = [inside().trim(), node.getAttribute('href') ?? ''];
      return href && href !== words ? `[${words}](${href})` : words;
    }
    case 'IMG':
      return `![${node.getAttribute('alt') ?? ''}](${node.getAttribute('src') ?? ''})`;
    case 'SPAN':
    case 'FONT':
    case 'U':
      return inside();
    default:
      // A block met where a line was expected: its own blocks, on lines of their own.
      return BLOCKS.has(node.tagName) ? `\n${block(node)}\n` : node.outerHTML;
  }
}

/** One element, as a block of Markdown. */
function block(node: HTMLElement): string {
  const tag = node.tagName;
  if (/^H[1-6]$/.test(tag)) return `${'#'.repeat(Number(tag[1]))} ${inline(node).trim()}`;
  if (tag === 'PRE') return `\`\`\`\n${(node.textContent ?? '').replace(/\n$/, '')}\n\`\`\``;
  if (tag === 'BLOCKQUOTE') {
    return blocks(node)
      .split('\n')
      .map((line) => `> ${line}`)
      .join('\n');
  }
  if (tag === 'UL' || tag === 'OL') {
    return [...node.children]
      .filter((item) => item.tagName === 'LI')
      .map((item, at) => {
        const mark = tag === 'OL' ? `${at + 1}. ` : '- ';
        const [first, ...rest] = blocks(item as HTMLElement).split('\n');
        // What follows an item's first line is set in under it.
        return [mark + first, ...rest.map((line) => (line ? ' '.repeat(mark.length) + line : line))].join(
          '\n',
        );
      })
      .join('\n');
  }
  return blocks(node);
}

/** The children of an element, as blocks of Markdown with an empty line between them. */
function blocks(node: HTMLElement): string {
  const out: string[] = [];
  let line = '';
  const flush = () => {
    if (line.trim()) out.push(line.replace(/[ \t]+\n/g, '\n').trim());
    line = '';
  };
  for (const child of node.childNodes) {
    if (child instanceof HTMLElement && BLOCKS.has(child.tagName)) {
      flush();
      const made = block(child);
      if (made.trim()) out.push(made);
    } else if (child instanceof HTMLElement && ['TABLE', 'HR', 'IFRAME', 'VIDEO'].includes(child.tagName)) {
      flush();
      out.push(child.outerHTML);
    } else line += inline(child);
  }
  flush();
  return out.join('\n\n');
}

/** A ticket's part, kept as HTML, as the Markdown you change it in. */
export function toMarkdown(html: string): string {
  const box = document.createElement('template');
  box.innerHTML = html;
  const holder = document.createElement('div');
  holder.append(box.content);
  return blocks(holder).trim();
}

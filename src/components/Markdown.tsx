import { Fragment, type ReactNode } from 'react';
import { parseMarkdown, type Inline } from '../lib/markdown';

interface MarkdownProps {
  source: string;
  /** Called for links the host can open; other links render as plain text. */
  onOpenLink?: (href: string) => void;
  canOpen?: (href: string) => boolean;
}

export function Markdown({ source, onOpenLink, canOpen }: MarkdownProps) {
  const renderInline = (nodes: Inline[]): ReactNode =>
    nodes.map((node, index) => {
      switch (node.type) {
        case 'text':
          return <Fragment key={index}>{node.text}</Fragment>;
        case 'strong':
          return <strong key={index}>{renderInline(node.children)}</strong>;
        case 'em':
          return <em key={index}>{renderInline(node.children)}</em>;
        case 'code':
          return <code key={index}>{node.text}</code>;
        case 'link':
          return onOpenLink && canOpen?.(node.href) ? (
            <button key={index} type="button" className="md-link" onClick={() => onOpenLink(node.href)}>
              {renderInline(node.children)}
            </button>
          ) : (
            <span key={index} title={node.href}>
              {renderInline(node.children)}
            </span>
          );
      }
    });

  const blocks = parseMarkdown(source);
  if (!blocks.length) return <p className="muted">Esta versão não tem notas publicadas.</p>;
  return (
    <div className="markdown">
      {blocks.map((block, index) => {
        switch (block.type) {
          case 'heading':
            return block.level === 1 ? <h3 key={index}>{renderInline(block.children)}</h3> : <h4 key={index}>{renderInline(block.children)}</h4>;
          case 'paragraph':
            return <p key={index}>{renderInline(block.children)}</p>;
          case 'rule':
            return <hr key={index} />;
          case 'list': {
            const items = block.items.map((item, itemIndex) => <li key={itemIndex}>{renderInline(item)}</li>);
            return block.ordered ? <ol key={index}>{items}</ol> : <ul key={index}>{items}</ul>;
          }
        }
      })}
    </div>
  );
}

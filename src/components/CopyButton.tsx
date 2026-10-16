import { useEffect, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { copyText } from '../lib/clipboard';
import { cx } from '../lib/cx';
import { useToast } from '../hooks/useToasts';
import { errorMessage } from '../types';

interface CopyButtonProps {
  value: string;
  /** What is being copied, for the accessible label: "Copiar endereço". */
  label: string;
  /** Visible text; omit for an icon-only button. */
  text?: string;
  className?: string;
}

/** Copies on click and confirms in place with a check mark for a moment. */
export function CopyButton({ value, label, text, className }: CopyButtonProps) {
  const toast = useToast();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const onClick = () => {
    copyText(value)
      .then(() => setCopied(true))
      .catch(reason => toast.error(errorMessage(reason)));
  };

  const Icon = copied ? Check : Copy;
  const accessibleLabel = copied ? 'Copiado' : `Copiar ${label}`;
  return (
    <button
      type="button"
      className={cx(text ? 'copy-btn' : 'icon-btn icon-btn-ghost icon-btn-sm', copied && 'is-copied', className)}
      onClick={onClick}
      aria-label={text ? undefined : accessibleLabel}
      title={accessibleLabel}
    >
      <Icon aria-hidden />
      {text ? <span aria-live="polite">{copied ? 'Copiado' : text}</span> : null}
    </button>
  );
}

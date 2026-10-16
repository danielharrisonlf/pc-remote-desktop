import { useEffect, useId, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { cx } from '../lib/cx';
import { IconButton } from './Button';

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  /** `drawer` slides in from the right edge; `dialog` is centered. */
  kind?: 'dialog' | 'drawer';
  size?: 'sm' | 'md' | 'lg';
  className?: string;
}

/**
 * Wraps the native <dialog> element: `showModal()` gives focus containment,
 * an inert background and Escape handling without a custom focus trap.
 * Content mounts only while open so each opening starts from fresh state.
 */
export function Dialog({ open, onClose, title, description, children, footer, kind = 'dialog', size = 'md', className }: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const returnFocus = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
      returnFocus.current?.focus();
    }
  }, [open]);

  return (
    <dialog
      ref={ref}
      className={cx(kind, `${kind}-${size}`, className)}
      aria-labelledby={titleId}
      aria-describedby={description ? descriptionId : undefined}
      onCancel={event => {
        event.preventDefault();
        onClose();
      }}
      onClick={event => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      {open ? (
        <div className="dialog-surface">
          <header className="dialog-header">
            <div>
              <h2 id={titleId}>{title}</h2>
              {description ? (
                <p id={descriptionId} className="dialog-description">
                  {description}
                </p>
              ) : null}
            </div>
            <IconButton label="Fechar" icon={X} onClick={onClose} />
          </header>
          <div className="dialog-body">{children}</div>
          {footer ? <footer className="dialog-footer">{footer}</footer> : null}
        </div>
      ) : null}
    </dialog>
  );
}

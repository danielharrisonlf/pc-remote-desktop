import type { ReactNode } from 'react';
import { Button } from './Button';
import { Dialog } from './Dialog';

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  children: ReactNode;
  confirmLabel: string;
  busy?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

/** Destructive confirmation. Cancel comes first so Enter on open never destroys. */
export function ConfirmDialog({ open, title, children, confirmLabel, busy, onConfirm, onClose }: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      size="sm"
      title={title}
      footer={
        <>
          <Button onClick={onClose} disabled={busy} autoFocus>
            Cancelar
          </Button>
          <Button variant="danger" loading={busy} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <div className="confirm-body">{children}</div>
    </Dialog>
  );
}

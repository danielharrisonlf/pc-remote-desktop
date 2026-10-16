import { useLayoutEffect, useRef } from 'react';
import { CircleAlert, CircleCheck, Info, X } from 'lucide-react';
import { useToastState, type ToastTone } from '../hooks/useToasts';

const icons: Record<ToastTone, typeof Info> = { success: CircleCheck, error: CircleAlert, info: Info };

/**
 * Bottom-right stack that overlays content, so feedback never pushes the page
 * down. It is a manual popover: re-showing it on each change moves it to the
 * top of the top layer, above an open modal dialog or drawer.
 */
export function ToastViewport() {
  const { toasts, dismiss, pause, resume } = useToastState();
  const ref = useRef<HTMLElement>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element || typeof element.showPopover !== 'function') return;
    try {
      if (element.matches(':popover-open')) element.hidePopover();
      if (toasts.length) element.showPopover();
    } catch {
      // Popover unsupported or detached: the fixed-position fallback still shows.
    }
  }, [toasts]);

  return (
    <section ref={ref} className="toast-viewport" aria-label="Notificações" popover="manual">
      {toasts.map(toast => {
        const Icon = icons[toast.tone];
        return (
          <div
            key={toast.id}
            className={`toast toast-${toast.tone}`}
            role={toast.tone === 'error' ? 'alert' : 'status'}
            onMouseEnter={() => pause(toast.id)}
            onMouseLeave={() => resume(toast.id)}
            onFocus={() => pause(toast.id)}
            onBlur={() => resume(toast.id)}
          >
            <Icon className="toast-icon" aria-hidden />
            <p className="toast-message">{toast.message}</p>
            {toast.action ? (
              <button
                type="button"
                className="toast-action"
                onClick={() => {
                  toast.action?.run();
                  dismiss(toast.id);
                }}
              >
                {toast.action.label}
              </button>
            ) : null}
            <button type="button" className="toast-close" aria-label="Dispensar notificação" onClick={() => dismiss(toast.id)}>
              <X aria-hidden />
            </button>
          </div>
        );
      })}
    </section>
  );
}

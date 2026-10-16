import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from 'react';

export type ToastTone = 'success' | 'error' | 'info';

export interface Toast {
  id: number;
  tone: ToastTone;
  message: string;
  action?: { label: string; run: () => void };
}

interface ToastApi {
  success: (message: string) => void;
  error: (message: string) => void;
  info: (message: string, action?: Toast['action']) => void;
}

interface ToastContextValue extends ToastApi {
  toasts: Toast[];
  dismiss: (id: number) => void;
  pause: (id: number) => void;
  resume: (id: number) => void;
}

const DURATION: Record<ToastTone, number> = { success: 3500, info: 6000, error: 8000 };
const MAX_VISIBLE = 3;
const ToastContext = createContext<ToastContextValue | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const current = useRef<Toast[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, number>());

  const commit = useCallback((next: Toast[]) => {
    current.current = next;
    setToasts(next);
  }, []);

  const clearTimer = useCallback((id: number) => {
    window.clearTimeout(timers.current.get(id));
    timers.current.delete(id);
  }, []);

  const dismiss = useCallback(
    (id: number) => {
      clearTimer(id);
      commit(current.current.filter(toast => toast.id !== id));
    },
    [clearTimer, commit],
  );

  const schedule = useCallback(
    (id: number, tone: ToastTone) => {
      window.clearTimeout(timers.current.get(id));
      timers.current.set(id, window.setTimeout(() => dismiss(id), DURATION[tone]));
    },
    [dismiss],
  );

  const push = useCallback(
    (tone: ToastTone, message: string, action?: Toast['action']) => {
      // Repeating the same message refreshes it instead of stacking duplicates.
      const duplicate = current.current.find(toast => toast.message === message && toast.tone === tone);
      if (duplicate) {
        schedule(duplicate.id, tone);
        return;
      }
      const id = nextId.current++;
      const next = [...current.current, { id, tone, message, action }];
      while (next.length > MAX_VISIBLE) clearTimer(next.shift()!.id);
      schedule(id, tone);
      commit(next);
    },
    [clearTimer, commit, schedule],
  );

  const pause = useCallback((id: number) => window.clearTimeout(timers.current.get(id)), []);

  const resume = useCallback(
    (id: number) => {
      const toast = current.current.find(item => item.id === id);
      if (toast) schedule(id, toast.tone);
    },
    [schedule],
  );

  const value = useMemo<ToastContextValue>(
    () => ({
      toasts,
      dismiss,
      pause,
      resume,
      success: message => push('success', message),
      error: message => push('error', message),
      info: (message, action) => push('info', message, action),
    }),
    [toasts, dismiss, pause, resume, push],
  );

  return <ToastContext.Provider value={value}>{children}</ToastContext.Provider>;
}

export function useToastState(): ToastContextValue {
  const context = useContext(ToastContext);
  if (!context) throw new Error('useToastState must be used inside ToastProvider');
  return context;
}

export function useToast(): ToastApi {
  const { success, error, info } = useToastState();
  return useMemo(() => ({ success, error, info }), [success, error, info]);
}

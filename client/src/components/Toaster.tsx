/**
 * In-app themed toaster — the Harbor's OWN notification surface (not the OS
 * notification plugin). Used for partner-activity events like "Taylor abriu
 * Valorant". Lives at the top of the app window, ocean-styled, smooth slide-in
 * + auto-dismiss. `notify.ts` (OS notifications) remains for
 * presence/message categories.
 *
 * Mount <ToastProvider> once near the root; any descendant calls useToast().
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

export interface Toast {
  id: string;
  title: string;
  body?: string;
  icon?: string;
  duration?: number;
}

interface ToastCtx {
  push: (t: Omit<Toast, "id">) => void;
  dismiss: (id: string) => void;
}

const Ctx = createContext<ToastCtx | null>(null);

export function useToast(): ToastCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error("useToast must be used within a ToastProvider");
  return c;
}

let seq = 0;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timers = useRef<Map<string, number>>(new Map());

  const dismiss = useCallback((id: string) => {
    setToasts((t) => t.filter((x) => x.id !== id));
    const h = timers.current.get(id);
    if (h) {
      clearTimeout(h);
      timers.current.delete(id);
    }
  }, []);

  const push = useCallback(
    (t: Omit<Toast, "id">) => {
      const id = `toast-${Date.now()}-${seq++}`;
      const duration = t.duration ?? 4500;
      setToasts((prev) => [...prev, { ...t, id }]);
      if (duration > 0) {
        const h = window.setTimeout(() => dismiss(id), duration);
        timers.current.set(id, h);
      }
    },
    [dismiss],
  );

  // Clear any pending timers on unmount.
  useEffect(
    () => () => {
      timers.current.forEach((h) => clearTimeout(h));
      timers.current.clear();
    },
    [],
  );

  return (
    <Ctx.Provider value={{ push, dismiss }}>
      {children}
      <div className="toast-region pointer-events-none fixed top-3 right-3 z-50 flex flex-col gap-2">
        {toasts.map((t) => (
          <div
            key={t.id}
            className="toast-enter pointer-events-auto flex items-start gap-2 bg-harbor-ice/90 backdrop-blur text-harbor-deep rounded-2xl shadow-lg px-3 py-2 max-w-[16rem]"
            role="status"
          >
            {t.icon && <span className="text-lg leading-5">{t.icon}</span>}
            <div className="flex-1 min-w-0">
              <p className="text-sm font-semibold text-harbor-deep">{t.title}</p>
              {t.body && <p className="text-xs text-harbor-deep/70 mt-0.5">{t.body}</p>}
            </div>
            <button
              className="text-harbor-deep/40 hover:text-harbor-deep text-xs leading-5"
              onClick={() => dismiss(t.id)}
              aria-label="Fechar"
            >
              ✕
            </button>
          </div>
        ))}
      </div>
    </Ctx.Provider>
  );
}

export default ToastProvider;

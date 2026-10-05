export interface ConfirmOptions {
  title: string;
  message?: string;
  confirmLabel?: string;
  /** Irreversible: the confirm button is styled as dangerous. */
  danger?: boolean;
  /** The user must type this exactly before confirming. */
  requireText?: string;
}

type Pending = ConfirmOptions & { resolve: (ok: boolean) => void };

export const confirmState = $state<{ current: Pending | null }>({ current: null });

/** In-page replacement for window.confirm; rendered by ConfirmDialog in App. */
export function ask(opts: ConfirmOptions): Promise<boolean> {
  confirmState.current?.resolve(false);
  return new Promise((resolve) => {
    confirmState.current = { ...opts, resolve };
  });
}

export function settle(ok: boolean) {
  const p = confirmState.current;
  confirmState.current = null;
  p?.resolve(ok);
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

type Listener = () => void;
const unauthorizedListeners: Listener[] = [];

export function onUnauthorized(fn: Listener) {
  unauthorizedListeners.push(fn);
}

export async function api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const resp = await fetch(path, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (resp.status === 204) return undefined as T;
  const text = await resp.text();
  let data: unknown = text;
  try {
    data = JSON.parse(text);
  } catch {
    /* plain text response */
  }
  if (!resp.ok) {
    if (resp.status === 401) unauthorizedListeners.forEach((f) => f());
    const msg = (data as { error?: string })?.error ?? `${resp.status} ${resp.statusText}`;
    throw new ApiError(resp.status, msg);
  }
  return data as T;
}

export const get = <T>(path: string) => api<T>("GET", path);
export const post = <T>(path: string, body: unknown = {}) => api<T>("POST", path, body);

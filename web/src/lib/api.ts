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

// The server's generic errors are terse English; everything else already arrives in Chinese.
const GENERIC: Record<string, string> = {
  "not found": "内容不存在或已被删除，请刷新页面",
  unauthorized: "登录已失效，请重新登录",
  "internal error": "服务出错了，请稍后重试；如果一直失败，查看服务端日志",
};

export async function api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  let resp: Response;
  try {
    resp = await fetch(path, {
      method,
      credentials: "same-origin",
      headers: body === undefined ? {} : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "连不上 Galley 服务，请检查服务是否在运行、网络是否正常");
  }
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
    const msg = (data as { error?: string })?.error;
    throw new ApiError(resp.status, msg ? (GENERIC[msg] ?? msg) : `请求失败（HTTP ${resp.status}），请稍后重试`);
  }
  return data as T;
}

export const get = <T>(path: string) => api<T>("GET", path);
export const post = <T>(path: string, body: unknown = {}) => api<T>("POST", path, body);
export const del = <T>(path: string) => api<T>("DELETE", path);

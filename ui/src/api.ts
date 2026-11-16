export type Format = "messageFormat" | "i18next" | "nestedJson";

export interface ProjectInfo {
  name: string;
  projectDir: string;
  baseLocale: string;
  locales: string[];
  format: Format;
  files: { namespace: string | null; pathPattern: string }[];
  gitBranch: string | null;
}

export interface Message {
  id: string;
  variants: string[];
  values: Record<string, Record<string, string>>;
}

export type IssueKind = "missing" | "missingPlural" | "missingVariable" | "unknownVariable";

export interface Issue {
  id: string;
  locale: string;
  variant: string;
  kind: IssueKind;
  message: string;
  variable?: string;
}

export interface MessagesResponse {
  messages: Message[];
  issues: Issue[];
  stats: Record<string, { translated: number; total: number }>;
}

export interface Edit {
  id: string;
  variant: string;
  locale: string;
  value: string | null;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", "x-inlocal": "1", ...init?.headers },
  });
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new Error(body?.error ?? `${res.status} ${res.statusText}`);
  }
  return res.json();
}

export const api = {
  project: () => request<ProjectInfo>("/api/project"),
  messages: () => request<MessagesResponse>("/api/messages"),
  save: (edits: Edit[]) =>
    request<MessagesResponse>("/api/save", { method: "POST", body: JSON.stringify({ edits }) }),
};

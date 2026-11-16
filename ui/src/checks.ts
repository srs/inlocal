// Client-side copy of the server checks, used for fields with unsaved edits so
// warnings update while you type. The server stays the source of truth.
import type { Format, Issue, Message } from "./api";

export function variableRegex(format: Format): RegExp {
  return format === "i18next"
    ? /\{\{\s*-?\s*([A-Za-z_][\w.]*)[^}]*\}\}/g
    : /\{\s*([A-Za-z_]\w*)\s*[,}]/g;
}

export function variables(text: string, format: Format): string[] {
  const out = new Set<string>();

  for (const m of text.matchAll(variableRegex(format))) out.add(m[1]);

  return [...out];
}

export function formatVariable(name: string, format: Format): string {
  return format === "i18next" ? `{{${name}}}` : `{${name}}`;
}

export function displayVariant(variant: string): string {
  return variant.split("=").pop() ?? variant;
}

export function checkField(
  msg: Message,
  locale: string,
  variant: string,
  text: string,
  baseLocale: string,
  format: Format,
  hasAnyOther: boolean,
): Issue[] {
  if (locale === baseLocale) return [];

  const source = msg.values[baseLocale]?.[variant];

  if (!source) return [];

  const base = { id: msg.id, locale, variant };

  if (!text) {
    return variant && hasAnyOther
      ? [{ ...base, kind: "missingPlural", message: `Missing plural form “${displayVariant(variant)}”` }]
      : [{ ...base, kind: "missing", message: "Missing translation" }];
  }

  const expected = variables(source, format);
  const actual = variables(text, format);

  return [
    ...expected
      .filter((v) => !actual.includes(v))
      .map((v) => ({ ...base, kind: "missingVariable" as const, variable: v, message: `Missing variable ${formatVariable(v, format)}` })),
    ...actual
      .filter((v) => !expected.includes(v))
      .map((v) => ({ ...base, kind: "unknownVariable" as const, variable: v, message: `Unknown variable ${formatVariable(v, format)}` })),
  ];
}

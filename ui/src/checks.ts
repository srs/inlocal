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

// -- Plural forms per locale --------------------------------------------------
//
// Languages differ in plural forms (Ukrainian adds few/many; an ordinal can be
// a plain string in Norwegian), so each locale has its own. Mirrors the
// server's `Message::variants_for` and `check_message`.

const FORM_ORDER = ["zero", "one", "two", "few", "many", "other", "*"];

export function sortVariants(variants: string[]): string[] {
  const rank = (v: string) => {
    const i = FORM_ORDER.indexOf(displayVariant(v));

    return i === -1 ? FORM_ORDER.length : i;
  };

  return [...variants].sort((a, b) => rank(a) - rank(b));
}

export function isCatchAll(variant: string): boolean {
  const form = displayVariant(variant);

  return form === "*" || form === "other";
}

/** The forms a message has in a locale: its own, or the base locale's. */
export function variantsFor(msg: Message, locale: string): string[] {
  const own = Object.keys(msg.values[locale] ?? {});

  return own.length > 0 ? sortVariants(own) : msg.variants;
}

export function isPlural(msg: Message): boolean {
  return msg.variants.some((v) => v !== "");
}

type ValueOf = (m: Message, locale: string, variant: string) => string;

/** Text to translate a form from, falling back to the catch-all form. */
export function sourceText(msg: Message, variant: string, locale: string, valueOf: ValueOf): string {
  const same = valueOf(msg, locale, variant);

  if (same) return same;

  const forms = variantsFor(msg, locale);
  const fallback = forms.find(isCatchAll) ?? forms[forms.length - 1];

  return fallback !== undefined ? valueOf(msg, locale, fallback) : "";
}

/** All issues for one message in one locale, using the current texts. */
export function checkLocale(
  msg: Message,
  locale: string,
  baseLocale: string,
  format: Format,
  valueOf: ValueOf,
): Issue[] {
  if (locale === baseLocale) return [];

  const baseTexts = msg.variants.map((v) => valueOf(msg, baseLocale, v));

  if (baseTexts.every((t) => !t)) return [];

  const forms = variantsFor(msg, locale);
  const filled = forms.filter((v) => valueOf(msg, locale, v));
  const issue = (variant: string) => ({ id: msg.id, locale, variant });

  if (filled.length === 0) {
    return forms.map((v) => ({ ...issue(v), kind: "missing" as const, message: "Missing translation" }));
  }

  const missingForm = (v: string) => ({
    ...issue(v),
    kind: "missingPlural" as const,
    message: `Missing plural form “${displayVariant(v)}”`,
  });

  const issues: Issue[] = forms.filter((v) => !valueOf(msg, locale, v)).map(missingForm);

  const plural = forms.some((v) => v !== "");
  const baseCatchAll = msg.variants.find(isCatchAll);

  if (plural && !forms.some(isCatchAll) && baseCatchAll !== undefined) {
    issues.push(missingForm(baseCatchAll));
  }

  // Variables, across all forms.
  const expected = new Set(baseTexts.flatMap((t) => variables(t, format)));
  const actual = new Set(filled.flatMap((v) => variables(valueOf(msg, locale, v), format)));
  const anchor = forms.find(isCatchAll) ?? filled[0];

  for (const v of expected) {
    if (!actual.has(v)) {
      issues.push({ ...issue(anchor), kind: "missingVariable", variable: v, message: `Missing variable ${formatVariable(v, format)}` });
    }
  }

  for (const v of actual) {
    if (!expected.has(v)) {
      issues.push({ ...issue(anchor), kind: "unknownVariable", variable: v, message: `Unknown variable ${formatVariable(v, format)}` });
    }
  }

  return issues;
}

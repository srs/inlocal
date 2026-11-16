import { type ReactNode, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle, CircleDashed, Copy, FileCode2, Folder, GitBranch, ArrowLeftRight, ChevronDown, Languages, LayoutList, List, Pencil, Plus, Radio, Search, Table2, Undo2,
} from "lucide-react";

import { api, type Edit, type Issue, type Message, type MessagesResponse, type ProjectInfo } from "./api";
import { checkField, displayVariant, formatVariable, variables } from "./checks";

type Filter = "all" | "attention" | "missing" | "changed";
type Status = "ok" | "warn" | "missing" | "changed";
type View = "focus" | "table";

function loadView(): View {
  try {
    return localStorage.getItem("inlocal.view") === "table" ? "table" : "focus";
  } catch {
    return "focus";
  }
}

const SEP = "\u0000";
const fieldKey = (id: string, variant: string, locale: string) => [id, variant, locale].join(SEP);

export function App() {
  const [project, setProject] = useState<ProjectInfo | null>(null);
  const [data, setData] = useState<MessagesResponse | null>(null);
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [filter, setFilter] = useState<Filter>("all");
  const [namespace, setNamespace] = useState<string | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [externalChange, setExternalChange] = useState(false);
  const [live, setLive] = useState(false);
  const [view, setViewState] = useState<View>(loadView);
  const [compareFrom, setCompareFrom] = useState<string | null>(null);
  const [compareTo, setCompareTo] = useState<string | null>(null);
  const [onlyMissing, setOnlyMissing] = useState(false);
  const setView = useCallback((v: View) => {
    setViewState(v);

    try {
      localStorage.setItem("inlocal.view", v);
    } catch {
      /* storage unavailable */
    }
  }, []);

  const searchRef = useRef<HTMLInputElement>(null);
  const editsRef = useRef(edits);
  editsRef.current = edits;

  const reload = useCallback(async () => {
    try {
      setData(await api.messages());
      setExternalChange(false);
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
  }, []);

  useEffect(() => {
    api.project().then(setProject).catch((e) => setError(String(e.message)));
    reload();
  }, [reload]);

  // Live reload when files change on disk.
  useEffect(() => {
    const es = new EventSource("/api/events");
    es.onopen = () => setLive(true);
    es.onerror = () => setLive(false);
    es.addEventListener("changed", () => {
      if (Object.keys(editsRef.current).length === 0) reload();

      else setExternalChange(true);
    });

    return () => es.close();
  }, [reload]);

  const base = project?.baseLocale ?? "";
  const format = project?.format ?? "messageFormat";
  const locales = useMemo(
    () => (project ? [base, ...project.locales.filter((l) => l !== base)] : []),
    [project, base],
  );

  const fromLocale = compareFrom && locales.includes(compareFrom) ? compareFrom : base;
  const toLocale =
    compareTo === "*" ? "*"
    : compareTo && locales.includes(compareTo) && compareTo !== fromLocale ? compareTo
    : locales.find((l) => l !== fromLocale) ?? fromLocale;

  const original = useCallback(
    (m: Message, locale: string, variant: string) => m.values[locale]?.[variant] ?? "",
    [],
  );

  const valueOf = useCallback(
    (m: Message, locale: string, variant: string) => {
      const k = fieldKey(m.id, variant, locale);

      return k in edits ? edits[k] : original(m, locale, variant);
    },
    [edits, original],
  );

  // Issues per message: server issues, replaced by client checks for edited fields.
  const issuesById = useMemo(() => {
    const map = new Map<string, Issue[]>();

    if (!data) return map;

    for (const i of data.issues) {
      if (fieldKey(i.id, i.variant, i.locale) in edits) continue;

      map.set(i.id, [...(map.get(i.id) ?? []), i]);
    }

    for (const k of Object.keys(edits)) {
      const [id, variant, locale] = k.split(SEP);
      const msg = data.messages.find((m) => m.id === id);

      if (!msg) continue;

      const hasAny = msg.variants.some((v) => v !== variant && valueOf(msg, locale, v));
      const found = checkField(msg, locale, variant, edits[k], base, format, hasAny);

      if (found.length) map.set(id, [...(map.get(id) ?? []), ...found]);
    }

    return map;
  }, [data, edits, base, format, valueOf]);

  const changedIds = useMemo(() => {
    const ids = new Set<string>();

    if (!data) return ids;

    for (const k of Object.keys(edits)) {
      const [id, variant, locale] = k.split(SEP);
      const msg = data.messages.find((m) => m.id === id);

      if (msg && edits[k] !== original(msg, locale, variant)) ids.add(id);
    }

    return ids;
  }, [data, edits, original]);

  const statusOf = useCallback(
    (id: string): Status => {
      if (changedIds.has(id)) return "changed";

      const issues = issuesById.get(id) ?? [];

      if (issues.some((i) => i.kind === "missing" || i.kind === "missingPlural")) return "missing";

      return issues.length ? "warn" : "ok";
    },
    [changedIds, issuesById],
  );

  const namespaces = useMemo(
    () => project?.files.map((f) => f.namespace).filter((n): n is string => !!n) ?? [],
    [project],
  );

  const visible = useMemo(() => {
    if (!data) return [];

    const q = query.trim().toLowerCase();

    return data.messages.filter((m) => {
      if (namespace && !m.id.startsWith(`${namespace}:`)) return false;

      const issues = issuesById.get(m.id) ?? [];

      if (filter === "attention" && issues.length === 0) return false;
      if (filter === "missing" && !issues.some((i) => i.kind === "missing" || i.kind === "missingPlural")) return false;
      if (filter === "changed" && !changedIds.has(m.id)) return false;

      if (view === "table" && onlyMissing && toLocale !== "*" &&
        !issues.some((i) => i.locale === toLocale && (i.kind === "missing" || i.kind === "missingPlural"))) return false;

      if (!q) return true;
      if (m.id.toLowerCase().includes(q)) return true;

      return Object.values(m.values).some((v) => Object.values(v).some((t) => t.toLowerCase().includes(q)));
    });
  }, [data, query, filter, namespace, issuesById, changedIds, view, onlyMissing, toLocale]);

  const counts = useMemo(() => {
    const all = data?.messages ?? [];

    return {
      all: all.length,
      attention: all.filter((m) => (issuesById.get(m.id) ?? []).length > 0).length,
      missing: all.filter((m) => (issuesById.get(m.id) ?? []).some((i) => i.kind === "missing" || i.kind === "missingPlural")).length,
      changed: changedIds.size,
    };
  }, [data, issuesById, changedIds]);

  useEffect(() => {
    if (visible.length && !visible.some((m) => m.id === selected)) setSelected(visible[0].id);
  }, [visible, selected]);

  const current = data?.messages.find((m) => m.id === selected) ?? null;

  const setField = (m: Message, locale: string, variant: string, value: string) => {
    const k = fieldKey(m.id, variant, locale);
    setEdits((prev) => {
      const next = { ...prev };

      if (value === original(m, locale, variant)) delete next[k];

      else next[k] = value;

      return next;
    });
  };

  const save = useCallback(async () => {
    const entries = Object.entries(editsRef.current);

    if (!entries.length || saving) return;

    const payload: Edit[] = entries.map(([k, value]) => {
      const [id, variant, locale] = k.split(SEP);

      return { id, variant, locale, value: value === "" ? null : value };
    });

    setSaving(true);

    try {
      setData(await api.save(payload));
      setEdits({});
      setExternalChange(false);
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setSaving(false);
    }
  }, [saving]);

  // Keyboard: ⌘K search, ⌘S save, ↑/↓ or j/k to move through the list.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;

      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();

        return;
      }

      if (mod && e.key === "\\") {
        e.preventDefault();
        setView(view === "focus" ? "table" : "focus");

        return;
      }

      if (mod && e.key.toLowerCase() === "s") {
        e.preventDefault();
        save();

        return;
      }

      const target = e.target as HTMLElement;
      const typing = target.tagName === "TEXTAREA" || (target.tagName === "INPUT" && target !== searchRef.current);

      if (typing || view === "table") return;

      if (e.key === "Escape" && target === searchRef.current) {
        searchRef.current?.blur();

        return;
      }

      const step = e.key === "ArrowDown" || (e.key === "j" && target !== searchRef.current) ? 1
        : e.key === "ArrowUp" || (e.key === "k" && target !== searchRef.current) ? -1 : 0;

      if (step && visible.length) {
        e.preventDefault();
        const i = visible.findIndex((m) => m.id === selected);
        const next = visible[Math.max(0, Math.min(visible.length - 1, i + step))];
        setSelected(next.id);
        document.getElementById(`row-${next.id}`)?.scrollIntoView({ block: "nearest" });
      }
    };

    window.addEventListener("keydown", onKey);

    return () => window.removeEventListener("keydown", onKey);
  }, [save, visible, selected, view, setView]);

  // Warn before closing with unsaved edits.
  useEffect(() => {
    const onUnload = (e: BeforeUnloadEvent) => {
      if (Object.keys(editsRef.current).length) e.preventDefault();
    };

    window.addEventListener("beforeunload", onUnload);

    return () => window.removeEventListener("beforeunload", onUnload);
  }, []);

  if (!project || !data) {
    return <div className="center">{error ? <span className="error">{error}</span> : "Loading…"}</div>;
  }

  const editCount = Object.keys(edits).length;
  const navItems: { key: Filter; label: string; icon: ReactNode }[] = [
    { key: "all", label: "All messages", icon: <List size={15} /> },
    { key: "attention", label: "Needs attention", icon: <AlertTriangle size={15} /> },
    { key: "missing", label: "Missing", icon: <CircleDashed size={15} /> },
    { key: "changed", label: "Changed", icon: <Pencil size={15} /> },
  ];

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <Languages size={17} />
          {project.name}
          <span className="path">/ project.inlang</span>
        </div>
        <label className="search">
          <Search size={14} />
          <input
            ref={searchRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search keys and text"
          />
          <span className="kbd">⌘K</span>
        </label>
        <div className="segmented" role="group" aria-label="View">
          <button className={view === "focus" ? "on" : ""} onClick={() => setView("focus")} title="Focus view (⌘\\)">
            <LayoutList size={14} /> Focus
          </button>
          <button className={view === "table" ? "on" : ""} onClick={() => setView("table")} title="Compare languages side by side (⌘\\)">
            <Table2 size={14} /> Compare
          </button>
        </div>
        {editCount > 0 && (
          <>
            <span className="unsaved">{editCount} unsaved</span>
            <button className="btn ghost" onClick={() => setEdits({})} title="Discard changes">
              <Undo2 size={14} />
            </button>
          </>
        )}
        <button className="btn primary" onClick={save} disabled={saving}>
          {saving ? "Saving…" : "Save"} <span className="kbd" style={{ borderColor: "transparent", color: "inherit", opacity: 0.6 }}>⌘S</span>
        </button>
      </header>

      <div style={{ display: "flex", flexDirection: "column", minHeight: 0 }}>
        {externalChange && (
          <div className="banner">
            <AlertTriangle size={14} /> Files changed on disk while you have unsaved edits.
            <button className="btn small" onClick={() => { setEdits({}); reload(); }}>Discard and reload</button>
          </div>
        )}
        {error && <div className="banner error"><AlertTriangle size={14} /> {error}</div>}

        <div className={`main ${view === "table" ? "wide" : ""}`} style={{ flex: 1 }}>
          <aside className="sidebar">
            {navItems.map((n) => (
              <button key={n.key} className={`nav ${filter === n.key ? "on" : ""}`} onClick={() => setFilter(n.key)}>
                {n.icon} {n.label} <span className="count">{counts[n.key]}</span>
              </button>
            ))}
            <div className="section">Locales</div>
            {locales.map((l) => {
              const s = data.stats[l];
              const pct = s && s.total ? Math.floor((s.translated / s.total) * 100) : 100;

              return (
                <button
                  key={l}
                  className={`nav ${hidden.has(l) ? "off" : ""}`}
                  title={hidden.has(l) ? "Show locale" : "Hide locale"}
                  onClick={() => {
                    if (l === base) return;

                    const next = new Set(hidden);

                    if (next.has(l)) next.delete(l);

                    else next.add(l);
                    setHidden(next);
                  }}
                >
                  <span className={`dot ${pct === 100 ? "ok" : pct >= 80 ? "warn" : "missing"}`} />
                  <span className="mono">{l}</span>
                  {l === base && <span style={{ color: "var(--text-3)", fontSize: 11 }}>source</span>}
                  <span className="count">{pct}%</span>
                </button>
              );
            })}
            {namespaces.length > 0 && (
              <>
                <div className="section">Namespaces</div>
                {namespaces.map((ns) => (
                  <button
                    key={ns}
                    className={`nav ${namespace === ns ? "on" : ""}`}
                    onClick={() => setNamespace(namespace === ns ? null : ns)}
                  >
                    <Folder size={15} /> {ns}
                  </button>
                ))}
              </>
            )}
          </aside>

          {view === "table" ? (
            <div className="compare">
              <div className="compare-bar">
                <label>
                  From
                  <span className="select">
                    <select value={fromLocale} onChange={(e) => setCompareFrom(e.target.value)}>
                      {locales.map((l) => <option key={l} value={l}>{l}{l === base ? " (source)" : ""}</option>)}
                    </select>
                    <ChevronDown size={13} aria-hidden="true" />
                  </span>
                </label>
                <button
                  className="btn ghost small"
                  title="Swap languages"
                  disabled={toLocale === "*"}
                  onClick={() => {
                    if (toLocale === "*") return;

                    setCompareFrom(toLocale);
                    setCompareTo(fromLocale);
                  }}
                >
                  <ArrowLeftRight size={14} />
                </button>
                <label>
                  To
                  <span className="select">
                    <select value={toLocale} onChange={(e) => setCompareTo(e.target.value)}>
                      {locales.filter((l) => l !== fromLocale).map((l) => <option key={l} value={l}>{l}</option>)}
                      <option value="*">All locales</option>
                    </select>
                    <ChevronDown size={13} aria-hidden="true" />
                  </span>
                </label>
                {toLocale !== "*" && (
                  <label className="check">
                    <input type="checkbox" checked={onlyMissing} onChange={(e) => setOnlyMissing(e.target.checked)} />
                    Only missing in <span className="mono">{toLocale}</span>
                  </label>
                )}
                <span className="compare-count">{visible.length} messages</span>
              </div>
            <TableView
              messages={visible}
              locales={toLocale === "*" ? locales.filter((l) => !hidden.has(l)) : [fromLocale, toLocale]}
              base={base}
              format={format}
              issuesById={issuesById}
              statusOf={statusOf}
              valueOf={valueOf}
              isChanged={(m, l, v) => fieldKey(m.id, v, l) in edits}
              setField={setField}
              onOpen={(id) => {
                setSelected(id);
                setView("focus");
              }}
            />
            </div>
          ) : (
          <>
          <nav className="list">
            {visible.length === 0 && <div className="empty">No messages match.</div>}
            {visible.map((m) => {
              const preview = valueOf(m, base, m.variants[m.variants.length - 1] ?? "");

              return (
                <div
                  key={m.id}
                  id={`row-${m.id}`}
                  className={`row ${m.id === selected ? "on" : ""}`}
                  onClick={() => setSelected(m.id)}
                >
                  <div className="id">
                    <span className={`dot ${statusOf(m.id)}`} />
                    <span className="mono">{m.id}</span>
                  </div>
                  <div className="preview">{preview || " "}</div>
                </div>
              );
            })}
          </nav>

          <main className="editor">
            {current ? (
              <MessageEditor
                msg={current}
                locales={locales.filter((l) => !hidden.has(l))}
                base={base}
                format={format}
                issues={issuesById.get(current.id) ?? []}
                valueOf={valueOf}
                isChanged={(l, v) => fieldKey(current.id, v, l) in edits}
                setField={setField}
              />
            ) : (
              <div className="empty">Select a message.</div>
            )}
          </main>
          </>
          )}
        </div>
      </div>

      <footer className="statusbar">
        {project.gitBranch && <span><GitBranch size={12} /> {project.gitBranch}</span>}
        <FilesStatus files={project.files} namespace={namespace} onSelect={setNamespace} />
        <span>{format}</span>
        <span className="right"><Radio size={12} /> {live ? "watching for changes" : "not watching"}</span>
      </footer>
    </div>
  );
}

function MessageEditor(props: {
  msg: Message;
  locales: string[];
  base: string;
  format: ProjectInfo["format"];
  issues: Issue[];
  valueOf: (m: Message, locale: string, variant: string) => string;
  isChanged: (locale: string, variant: string) => boolean;
  setField: (m: Message, locale: string, variant: string, value: string) => void;
}) {
  const { msg, locales, base, format, issues, valueOf, isChanged, setField } = props;
  const plural = msg.variants.length > 1 || (msg.variants[0] ?? "") !== "";
  const vars = [...new Set(msg.variants.flatMap((v) => variables(valueOf(msg, base, v), format)))];

  return (
    <>
      <div className="editor-head">
        <span className="title">{msg.id}</span>
        <button className="btn ghost small" title="Copy key" onClick={() => navigator.clipboard?.writeText(msg.id)}>
          <Copy size={13} />
        </button>
        {plural && <span className="pill" style={{ marginLeft: "auto" }}>plural</span>}
      </div>

      {locales.map((locale) => (
        <div key={locale} className="locale-row">
          <span className={`locale-tag ${locale === base ? "base" : ""}`}>{locale}</span>
          <div>
            {msg.variants.map((variant) => {
              const fieldIssues = issues.filter((i) => i.locale === locale && i.variant === variant);
              const source = valueOf(msg, base, variant);

              return (
                <div key={variant}>
                  <div className={plural ? "variant" : undefined}>
                    {plural && <span className="variant-name">{displayVariant(variant)}</span>}
                    <AutoField
                      value={valueOf(msg, locale, variant)}
                      changed={isChanged(locale, variant)}
                      placeholder={locale === base ? "Source text" : source || "Add translation"}
                      onChange={(v) => setField(msg, locale, variant, v)}
                    />
                  </div>
                  {fieldIssues.map((issue, i) => (
                    <div key={i} className="issue" style={plural ? { marginLeft: 60 } : undefined}>
                      <AlertTriangle size={13} /> {issue.message}
                      {(issue.kind === "missing" || issue.kind === "missingPlural") && source && (
                        <button className="btn small" onClick={() => setField(msg, locale, variant, source)}>
                          <Copy size={12} /> Copy from {base}
                        </button>
                      )}
                      {issue.kind === "missingVariable" && (
                        <button
                          className="btn small"
                          onClick={() => {
                            const name = issue.variable ?? "";
                            const cur = valueOf(msg, locale, variant);
                            setField(msg, locale, variant, `${cur}${cur.endsWith(" ") || !cur ? "" : " "}${formatVariable(name, format)}`);
                          }}
                        >
                          <Plus size={12} /> Insert variable
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      ))}

      <div className="vars">
        Variables: {vars.length ? vars.map((v) => <span key={v} className="var">{formatVariable(v, format)}</span>) : "none"}
      </div>
      <div className="hint">Empty fields are removed from the file on save. Only edited values change on disk.</div>
    </>
  );
}

function AutoField(props: {
  value: string;
  changed: boolean;
  warn?: boolean;
  title?: string;
  placeholder: string;
  onChange: (v: string) => void;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;

    if (!el) return;

    el.style.height = "auto";
    el.style.height = `${el.scrollHeight + 2}px`;
  }, [props.value]);

  return (
    <textarea
      ref={ref}
      rows={1}
      className={`field ${props.changed ? "changed" : props.warn ? "warn" : ""}`}
      title={props.title}
      value={props.value}
      placeholder={props.placeholder}
      spellCheck
      onChange={(e) => props.onChange(e.target.value)}
    />
  );
}

function TableView(props: {
  messages: Message[];
  locales: string[];
  base: string;
  format: ProjectInfo["format"];
  issuesById: Map<string, Issue[]>;
  statusOf: (id: string) => Status;
  valueOf: (m: Message, locale: string, variant: string) => string;
  isChanged: (m: Message, locale: string, variant: string) => boolean;
  setField: (m: Message, locale: string, variant: string, value: string) => void;
  onOpen: (id: string) => void;
}) {
  const { messages, locales, base, issuesById, statusOf, valueOf, isChanged, setField, onOpen } = props;

  if (messages.length === 0) return <div className="table-wrap"><div className="empty">No messages match.</div></div>;

  return (
    <div className="table-wrap">
      <table className="grid" style={{ minWidth: 260 + locales.length * 240 }}>
        <colgroup>
          <col style={{ width: 260 }} />
          {locales.map((l) => <col key={l} />)}
        </colgroup>
        <thead>
          <tr>
            <th className="key-col">Key</th>
            {locales.map((l) => (
              <th key={l}>
                <span className={`locale-tag ${l === base ? "base" : ""}`}>{l}</span>
                {l === base && <span className="th-note">source</span>}
              </th>
            ))}
          </tr>
        </thead>
        {messages.map((m) => {
          const issues = issuesById.get(m.id) ?? [];
          const plural = m.variants.length > 1 || (m.variants[0] ?? "") !== "";

          return (
            <tbody key={m.id} className="msg">
              {m.variants.map((variant, vi) => (
                <tr key={variant}>
                  {vi === 0 && (
                    <td className="key-col" rowSpan={m.variants.length}>
                      <button className="key-link" onClick={() => onOpen(m.id)} title="Open in focus view">
                        <span className={`dot ${statusOf(m.id)}`} />
                        <span className="mono">{m.id}</span>
                      </button>
                      {plural && <span className="pill" style={{ marginLeft: 15 }}>plural</span>}
                    </td>
                  )}
                  {locales.map((l) => {
                    const cellIssues = issues.filter((i) => i.locale === l && i.variant === variant);
                    const source = valueOf(m, base, variant);
                    const missing = cellIssues.some((i) => i.kind === "missing" || i.kind === "missingPlural");

                    return (
                      <td key={l}>
                        <div className="cell">
                          {plural && <span className="cell-variant">{displayVariant(variant)}</span>}
                          <AutoField
                            value={valueOf(m, l, variant)}
                            changed={isChanged(m, l, variant)}
                            warn={cellIssues.length > 0}
                            title={cellIssues.map((i) => i.message).join("\n") || undefined}
                            placeholder={l === base ? "Source text" : source || "Add translation"}
                            onChange={(v) => setField(m, l, variant, v)}
                          />
                          {cellIssues.length > 0 && (
                            <div className="cell-issue">
                              <AlertTriangle size={12} /> {cellIssues[0].message}
                              {cellIssues.length > 1 && ` +${cellIssues.length - 1}`}
                              {missing && source && (
                                <button className="btn ghost small" title={`Copy from ${base}`} onClick={() => setField(m, l, variant, source)}>
                                  <Copy size={12} />
                                </button>
                              )}
                            </div>
                          )}
                        </div>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          );
        })}
      </table>
    </div>
  );
}

/** Footer item for translation files. Shows the pattern when there is one
 *  (or a namespace is selected), otherwise a count that opens a list. */
function FilesStatus(props: {
  files: ProjectInfo["files"];
  namespace: string | null;
  onSelect: (ns: string) => void;
}) {
  const { files, namespace, onSelect } = props;
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;

    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };

    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);

    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);

  const active = namespace ? files.find((f) => f.namespace === namespace) : files.length === 1 ? files[0] : null;

  if (active && files.length === 1) {
    return (
      <span className="status-item" title={active.pathPattern}>
        <FileCode2 size={12} /> {active.pathPattern}
      </span>
    );
  }

  return (
    <div className="status-pop" ref={ref}>
      <button className="status-item status-btn" onClick={() => setOpen(!open)} aria-expanded={open}>
        <FileCode2 size={12} /> {active ? active.pathPattern : `${files.length} namespaces`}
      </button>
      {open && (
        <div className="popover" role="dialog" aria-label="Translation files">
          {files.map((f) => (
            <button
              key={f.pathPattern}
              className={`popover-row ${f.namespace === namespace ? "on" : ""}`}
              title={`Show only ${f.namespace}`}
              onClick={() => {
                if (f.namespace) onSelect(f.namespace);

                setOpen(false);
              }}
            >
              <span className="mono ns">{f.namespace ?? "default"}</span>
              <span className="mono path">{f.pathPattern}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

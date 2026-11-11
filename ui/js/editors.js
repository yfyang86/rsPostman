// Editors for collections / folders (auth, variables, scripts) and environments.
import { api } from "./api.js";
import { state, views, emit, collectionById, saveCollection, markDirty, reconcileTabs } from "./state.js";
import { h, clear, toast, guard, kvEditor, cleanRows, tabStrip, clone } from "./util.js";
import { findItem, scriptOf, setScript, countRequests } from "./model.js";
import { authEditor } from "./authui.js";

views.collection = (tab) => {
  const load = () => {
    const c = clone(collectionById(tab.collectionId));
    const target = tab.itemId ? findItem(c.item, tab.itemId).item : c;
    return { c, target };
  };
  let { c, target } = load();
  const isCollection = !tab.itemId;
  // working copy of the editable fields
  const draft = {
    name: isCollection ? c.info.name : target.name,
    description: (isCollection ? c.info.description : target.description) || "",
    auth: target.auth || null,
    variable: isCollection ? clone(c.variable || []) : [],
    pre: scriptOf(target.event, "prerequest"),
    test: scriptOf(target.event, "test"),
  };
  const dirty = () => markDirty(tab, true);
  const body = h("div");
  const strip = h("div");

  const scriptArea = (key, hint) => {
    const ta = h("textarea", { spellcheck: "false", style: "width:100%;min-height:260px", testid: "collection-script-" + key, "aria-label": hint, onInput: (e) => { draft[key] = e.target.value; dirty(); } });
    ta.value = draft[key];
    return [h("p.hint", hint), ta];
  };

  const draw = () => {
    clear(strip, tabStrip([
      { id: "overview", label: "Overview" },
      { id: "auth", label: "Authorization", badge: draft.auth && draft.auth.type !== "noauth" ? { cls: "", text: "●" } : null },
      isCollection ? { id: "variables", label: "Variables", count: draft.variable.filter((v) => v.key).length } : null,
      { id: "pre", label: "Pre-request", badge: draft.pre.trim() ? { cls: "", text: "●" } : null },
      { id: "tests", label: "Tests", badge: draft.test.trim() ? { cls: "", text: "●" } : null },
    ].filter(Boolean), tab.section, (id) => { tab.section = id; draw(); }, "collection-tab"));
    const s = tab.section;
    if (s === "overview") {
      const ta = h("textarea", { style: "width:100%;min-height:140px;font-family:inherit;font-size:13px", placeholder: "Description (shown in generated documentation)", testid: "collection-description", "aria-label": "Description", onInput: (e) => { draft.description = e.target.value; dirty(); } });
      ta.value = draft.description;
      clear(body, h("p.hint", isCollection ? `${countRequests(c.item)} requests · id ${c.info._postman_id}` : `${countRequests(target.item)} requests in this folder`), ta);
    } else if (s === "auth") {
      clear(body, h("p.hint", `Requests in this ${isCollection ? "collection" : "folder"} that use “Inherit auth from parent” get this authorization.`),
        authEditor(draft, { allowInherit: !isCollection, collectionId: tab.collectionId, onChange: () => { dirty(); } }));
    } else if (s === "variables") {
      clear(body, h("p.hint", "Collection variables are available to every request in the collection as {{name}}."),
        kvEditor(draft.variable, { onChange: dirty, testid: "collection-variables", keyLabel: "Variable", description: true, secretType: true }));
    } else if (s === "pre") {
      clear(body, scriptArea("pre", `Runs before every request in this ${isCollection ? "collection" : "folder"}.`));
    } else {
      clear(body, scriptArea("test", `Runs after every request in this ${isCollection ? "collection" : "folder"}.`));
    }
  };

  tab.save = () => guard(async () => {
    if (!draft.name.trim()) return toast("Name is required", "error");
    ({ c, target } = load());
    if (isCollection) {
      c.info.name = draft.name.trim();
      c.info.description = draft.description || undefined;
      c.variable = cleanRows(draft.variable);
    } else {
      target.name = draft.name.trim();
      target.description = draft.description || undefined;
    }
    if (draft.auth) target.auth = draft.auth;
    else delete target.auth;
    setScript(target, "prerequest", draft.pre);
    setScript(target, "test", draft.test);
    await saveCollection(c);
    tab.name = draft.name.trim();
    tab.dirty = false;
    emit("tabmeta");
    reconcileTabs();
    toast("Saved", "ok");
  });

  draw();
  return h("div.page", { testid: "collection-editor" },
    h("input.name-input", { type: "text", value: draft.name, "aria-label": "Name", testid: "collection-name", onInput: (e) => { draft.name = e.target.value; dirty(); } }),
    h("div.page-actions", h("button.btn.primary", { testid: "collection-save", onClick: () => tab.save() }, "Save"), h("span.muted", isCollection ? "Collection" : "Folder")),
    strip, h("div", { style: "padding-top:12px" }, body));
};

views.environment = (tab) => {
  const isGlobals = tab.envId === "globals";
  const env = tab.draft;
  env.values = env.values || [];
  const dirty = () => markDirty(tab, true);
  tab.save = () => guard(async () => {
    if (!isGlobals && !env.name.trim()) return toast("Name is required", "error");
    const payload = { ...env, values: env.values.filter((v) => v.key || v.value).map((v) => ({ key: v.key, value: v.value || "", enabled: v.enabled !== false, type: v.type })) };
    if (isGlobals) state.globals = await api.put("/globals", payload);
    else {
      const saved = await api.put("/environments/" + tab.envId, payload);
      const i = state.environments.findIndex((e) => e.id === tab.envId);
      if (i !== -1) state.environments[i] = saved;
      tab.name = saved.name;
    }
    tab.dirty = false;
    emit("environments");
    emit("tabmeta");
    toast("Saved", "ok");
  });
  return h("div.page", { testid: "environment-editor" },
    isGlobals ? h("h2", "Globals") : h("input.name-input", { type: "text", value: env.name, "aria-label": "Environment name", testid: "environment-name", onInput: (e) => { env.name = e.target.value; dirty(); } }),
    h("div.page-actions", h("button.btn.primary", { testid: "environment-save", onClick: () => tab.save() }, "Save"),
      h("span.muted", isGlobals ? "Global variables are available in every request, whatever environment is active." : "Use variables as {{name}} in URLs, headers, bodies and scripts.")),
    kvEditor(env.values, { enabledKey: "enabled", onChange: dirty, keyLabel: "Variable", testid: "environment-variables", secretType: true }),
    h("p.hint", "Precedence when names clash: local → iteration data → environment → collection → globals."),
    h("p.hint", "Tick “Secret” for passwords, tokens and keys: the value is masked here and in CLI listings. Values are stored unencrypted in the workspace folder, which only your user account can read."));
};

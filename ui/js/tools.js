// Toolbar tools. Sprint 3: import. Sprint 4 adds runner, mock servers, WebSocket, cookies, settings, code snippets.
import { api } from "./api.js";
import { state, tools, emit, openTab, requestTab } from "./state.js";
import { h, toast, guard, modal } from "./util.js";
import { normalizeRequest } from "./model.js";

export function importDialog() {
  const text = h("textarea", { style: "width:100%;min-height:200px", spellcheck: "false", placeholder: "Paste a curl command, Postman collection / environment JSON, OpenAPI (JSON or YAML) or HAR…", testid: "import-text", "aria-label": "Import text" });
  const file = h("input", { type: "file", accept: ".json,.yaml,.yml,.har,.txt", testid: "import-file", "aria-label": "Import file", onChange: async (e) => {
    const f = e.target.files[0];
    if (f) text.value = await f.text();
  } });
  modal({
    title: "Import",
    wide: true,
    testid: "import-dialog",
    body: [h("p.hint", "Supported: Postman Collection v2.0 / v2.1, Postman environments and globals, OpenAPI 3 / Swagger 2, HAR, curl."), h("div.field-row", file), text],
    buttons: [
      { label: "Cancel" },
      { label: "Import", primary: true, testid: "import-confirm", action: async () => {
        if (!text.value.trim()) return text.focus(), false;
        const r = await guard(() => api.post("/import", { text: text.value }));
        if (!r) return false;
        if (r.kind === "collection") {
          state.collections.push(r.collection);
          state.collections.sort((a, b) => a.info.name.toLowerCase().localeCompare(b.info.name.toLowerCase()));
          state.expanded.add(r.collection.info._postman_id);
          state.side = "collections";
          emit("collections");
          toast(`Imported collection “${r.collection.info.name}”`, "ok");
        } else if (r.kind === "environment") {
          state.environments.push(r.environment);
          emit("environments");
          toast(`Imported environment “${r.environment.name}”`, "ok");
        } else if (r.kind === "globals") {
          state.globals = r.globals;
          emit("environments");
          toast("Global variables merged", "ok");
        } else if (r.kind === "request") {
          const req = normalizeRequest(r.request);
          openTab(requestTab({ name: `${req.method} ${req.url.raw.replace(/^https?:\/\//, "").split("?")[0]}`.slice(0, 60), request: req, dirty: true }));
          toast("curl command imported into a new tab", "ok");
        }
        return true;
      } },
    ],
  });
}

tools.push({ id: "import", label: "Import", icon: "⇩", title: "Import collections, environments, OpenAPI, HAR or curl", action: importDialog });

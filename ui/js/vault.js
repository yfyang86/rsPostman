// Local vault: secrets encrypted at rest, unlocked with a master passphrase and used as
// {{vault:name}}. Values are write-only here – the backend never sends one back.
import { helpButton } from "./help.js";
import { api } from "./api.js";
import { state, views, emit, openSingleton } from "./state.js";
import { h, clear, toast, guard, confirmDialog, copyText, secretInput } from "./util.js";

export function setVaultStatus(status) {
  state.vault = status;
  emit("vault");
}

export function openVault() {
  openSingleton("vault", { name: "Vault" });
}

const passField = (id, label, testid) => {
  const s = secretInput({ id, testid, placeholder: label, "aria-label": label }, () => true);
  return s;
};

views.vault = (tab) => {
  const body = h("div");

  const act = async (fn, okMessage) => {
    const status = await guard(fn);
    if (!status) return false;
    setVaultStatus(status);
    if (okMessage) toast(okMessage, "ok");
    draw();
    return true;
  };

  const drawCreate = () => {
    const a = passField("vault-new-pass", "New passphrase (at least 8 characters)", "vault-new-pass");
    const b = passField("vault-new-pass2", "Repeat the passphrase", "vault-new-pass2");
    const submit = () => {
      if (a.input.value !== b.input.value) return toast("The passphrases do not match", "error");
      act(() => api.post("/vault/create", { passphrase: a.input.value }), "Vault created");
    };
    b.input.addEventListener("keydown", (e) => { if (e.key === "Enter") submit(); });
    clear(body,
      h("div.card",
        h("p", { style: "margin-top:0" }, "The vault keeps passwords, tokens and keys ", h("b", "encrypted on disk"), ". You unlock it with a passphrase when you start working; requests then use a secret as ", h("code.mono", "{{vault:name}}"), "."),
        h("div.form-grid", h("label", { for: "vault-new-pass" }, "Passphrase"), a.el, h("label", { for: "vault-new-pass2" }, "Repeat"), b.el),
        h("div.page-actions", h("button.btn.primary", { testid: "vault-create", onClick: submit }, "Create vault")),
        h("p.hint", "The passphrase is not stored anywhere and cannot be recovered. If it is lost, the secrets in the vault are lost.")));
  };

  const drawLocked = () => {
    const p = passField("vault-pass", "Passphrase", "vault-pass");
    const submit = () => act(() => api.post("/vault/unlock", { passphrase: p.input.value }), "Vault unlocked");
    p.input.addEventListener("keydown", (e) => { if (e.key === "Enter") submit(); });
    clear(body,
      h("div.card",
        h("div.field-row", h("span.status-pill.s4", { testid: "vault-state" }, "Locked"), h("span.muted", `${state.vault.names.length} secret${state.vault.names.length === 1 ? "" : "s"} stored`)),
        h("div.form-grid", h("label", { for: "vault-pass" }, "Passphrase"), p.el),
        h("div.page-actions", h("button.btn.primary", { testid: "vault-unlock", onClick: submit }, "Unlock")),
        h("p.hint", "While the vault is locked, requests that use {{vault:…}} are not sent.")),
      names(false));
    p.input.focus();
  };

  const names = (unlocked) => {
    const list = state.vault.names;
    if (!list.length) return h("div.placeholder", "The vault is empty.");
    return h("table.data", { testid: "vault-secrets" },
      h("thead", h("tr", h("th", "Secret"), h("th", "Use in requests as"), h("th", ""))),
      h("tbody", list.map((n) => h("tr", { testid: "vault-row" },
        h("td.k", n),
        h("td", h("code", `{{vault:${n}}}`), " ", h("button.btn.small", { onClick: () => copyText(`{{vault:${n}}}`) }, "Copy")),
        h("td", { style: "text-align:right;white-space:nowrap" }, unlocked ? h("button.btn.small.danger", { testid: "vault-delete", "aria-label": `Delete ${n}`, onClick: async () => {
          if (await confirmDialog(`Delete secret “${n}”?`, "Requests that use it will stop working.")) act(() => api.del("/vault/secrets/" + encodeURIComponent(n)));
        } }, "Delete") : null)))));
  };

  const drawUnlocked = () => {
    const name = h("input", { type: "text", id: "vault-name", placeholder: "name, e.g. github_token", "aria-label": "Secret name", testid: "vault-name", spellcheck: "false", autocomplete: "off" });
    const value = passField("vault-value", "Secret value", "vault-value");
    const add = async () => {
      const n = name.value.trim();
      if (!n || !value.input.value) return toast("Enter a name and a value", "error");
      const replacing = state.vault.names.includes(n);
      if (await act(() => api.put("/vault/secrets/" + encodeURIComponent(n), { value: value.input.value }), replacing ? `Secret “${n}” replaced` : `Secret “${n}” stored`)) {
        // draw() rebuilt the form; nothing to clear
      }
    };
    value.input.addEventListener("keydown", (e) => { if (e.key === "Enter") add(); });

    const cur = passField("vault-cur-pass", "Current passphrase", "vault-cur-pass");
    const next = passField("vault-next-pass", "New passphrase", "vault-next-pass");
    clear(body,
      h("div.card",
        h("div.field-row", h("span.status-pill.s2", { testid: "vault-state" }, "Unlocked"), h("span.muted", "Secrets are available to requests until you lock the vault or quit rsPostman."), h("span.spacer"),
          h("button.btn", { testid: "vault-lock", onClick: () => act(() => api.post("/vault/lock"), "Vault locked") }, "Lock now")),
        h("div.section-title", "Add or replace a secret"),
        h("div.field-row", { style: "margin:0" }, name, h("div", { style: "flex:1;min-width:200px" }, value.el), h("button.btn.primary", { testid: "vault-add", onClick: add }, "Store")),
        h("p.hint", "Stored values cannot be displayed again here; replace a secret by storing a new value under the same name. (`rspostman vault get NAME` prints one in a terminal.)")),
      names(true),
      h("details", { style: "margin-top:14px" }, h("summary.muted", { style: "cursor:pointer" }, "Change passphrase"),
        h("div.card", { style: "margin-top:8px" },
          h("div.form-grid", h("label", { for: "vault-cur-pass" }, "Current passphrase"), cur.el, h("label", { for: "vault-next-pass" }, "New passphrase"), next.el),
          h("div.page-actions", h("button.btn", { testid: "vault-passwd", onClick: () => act(() => api.post("/vault/passphrase", { current: cur.input.value, new: next.input.value }), "Passphrase changed") }, "Change passphrase")))));
  };

  const draw = () => {
    const v = state.vault;
    if (!v.exists) drawCreate();
    else if (!v.unlocked) drawLocked();
    else drawUnlocked();
  };

  tab.onShow = async () => {
    const status = await guard(() => api.get("/vault"));
    if (status) {
      const changed = JSON.stringify(status) !== JSON.stringify(state.vault);
      setVaultStatus(status);
      if (changed) draw();
    }
  };
  draw();
  return h("div.page", { testid: "vault" }, h("h2", "Vault", helpButton("vault-and-credentials")),
    h("div.sub", "Encrypted storage for credentials. Vault secrets are resolved only when a request is sent and never appear in the history, reports or scripts."), body);
};

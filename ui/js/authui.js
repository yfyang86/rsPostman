// Authorization editor shared by requests, folders and collections.
import { api } from "./api.js";
import { h, clear, guard, toast, secretInput, isReference } from "./util.js";
import { authParams, makeAuth } from "./model.js";

const TYPES = [
  ["inherit", "Inherit auth from parent"],
  ["noauth", "No Auth"],
  ["basic", "Basic Auth"],
  ["bearer", "Bearer Token"],
  ["apikey", "API Key"],
  ["digest", "Digest Auth"],
  ["oauth2", "OAuth 2.0"],
  ["oauth1", "OAuth 1.0"],
  ["awsv4", "AWS Signature"],
  ["jwt", "JWT Bearer"],
];

// [key, label, kind, options/placeholder]
const FIELDS = {
  basic: [["username", "Username"], ["password", "Password", "secret"]],
  bearer: [["token", "Token", "secret"]],
  apikey: [["key", "Key", "text", "X-API-Key"], ["value", "Value", "secret"], ["in", "Add to", "select", [["header", "Header"], ["query", "Query params"]]]],
  digest: [["username", "Username"], ["password", "Password", "secret"]],
  oauth2: [
    ["accessToken", "Access token", "secret"],
    ["headerPrefix", "Header prefix", "text", "Bearer"],
    ["addTokenTo", "Add token to", "select", [["header", "Request header"], ["queryParams", "Query params"]]],
  ],
  oauth1: [
    ["consumerKey", "Consumer key"], ["consumerSecret", "Consumer secret", "secret"], ["token", "Access token"], ["tokenSecret", "Token secret", "secret"],
    ["signatureMethod", "Signature method", "select", [["HMAC-SHA1", "HMAC-SHA1"], ["HMAC-SHA256", "HMAC-SHA256"], ["PLAINTEXT", "PLAINTEXT"]]],
    ["realm", "Realm"],
    ["addParamsToHeader", "Add auth data to", "select", [["true", "Request header"], ["false", "Query params"]]],
  ],
  awsv4: [["accessKey", "Access key"], ["secretKey", "Secret key", "secret"], ["region", "AWS region", "text", "us-east-1"], ["service", "Service name", "text", "execute-api"], ["sessionToken", "Session token", "secret"]],
  jwt: [
    ["algorithm", "Algorithm", "select", [["HS256", "HS256"], ["HS384", "HS384"], ["HS512", "HS512"]]],
    ["secret", "Secret", "secret"],
    ["isSecretBase64Encoded", "Secret is base64", "select", [["false", "No"], ["true", "Yes"]]],
    ["payload", "Payload (JSON)", "textarea", "{}"],
    ["headerPrefix", "Header prefix", "text", "Bearer"],
    ["addTokenTo", "Add token to", "select", [["header", "Request header"], ["queryParam", "Query param"]]],
  ],
};

const OAUTH2_GRANT = [
  ["grant_type", "Grant type", "select", [["client_credentials", "Client credentials"], ["password_credentials", "Password credentials"], ["refresh_token", "Refresh token"]]],
  ["accessTokenUrl", "Access token URL", "text", "https://example.com/oauth/token"],
  ["clientId", "Client ID"], ["clientSecret", "Client secret", "secret"],
  ["username", "Username"], ["password", "Password", "secret"],
  ["refreshToken", "Refresh token", "secret"],
  ["scope", "Scope", "text", "read write"],
  ["client_authentication", "Client authentication", "select", [["header", "Basic auth header"], ["body", "In request body"]]],
];

const HINTS = {
  inherit: "This item uses the authorization of its parent folder or collection.",
  noauth: "No authorization is sent with the request.",
  basic: "Tip: keep the password in the vault and reference it as {{vault:name}} – it is then encrypted on disk and the collection file contains no secret.",
  bearer: "Tip: keep the token in the vault and reference it as {{vault:name}} – it is then encrypted on disk and the collection file contains no secret.",
  apikey: "Tip: keep the key in the vault and reference it as {{vault:name}} – it is then encrypted on disk and the collection file contains no secret.",
  digest: "The request is retried automatically after the server's 401 challenge.",
  awsv4: "The request is signed with AWS Signature Version 4 when it is sent.",
  oauth1: "The OAuth 1.0a signature is generated when the request is sent.",
  jwt: "A signed JWT is generated for every request.",
};

/**
 * holder.auth is read and written in Postman format. opts: { allowInherit, onChange, collectionId }
 */
export function authEditor(holder, opts = {}) {
  let type = holder.auth ? holder.auth.type || "noauth" : opts.allowInherit === false ? "noauth" : "inherit";
  let params = authParams(holder.auth);
  const commit = () => {
    holder.auth = makeAuth(type, params);
    if (opts.onChange) opts.onChange();
  };
  const fieldsEl = h("div");

  const field = ([key, label, kind = "text", extra]) => {
    const id = "auth-" + key;
    let input;
    if (kind === "select") {
      input = h("select", { id, testid: id, onChange: (e) => { params[key] = e.target.value; commit(); } },
        extra.map(([v, l]) => h("option", { value: v, selected: (params[key] || extra[0][0]) === v }, l)));
    } else if (kind === "textarea") {
      input = h("textarea", { id, testid: id, placeholder: extra || "", spellcheck: "false", onInput: (e) => { params[key] = e.target.value; commit(); } });
      input.value = params[key] || "";
    } else if (kind === "secret") {
      // shown as dots unless it is only a {{variable}} reference; the eye button reveals it
      const s = secretInput({ id, testid: id, placeholder: extra || "value or {{variable}}", value: params[key] || "",
        onInput: (e) => { params[key] = e.target.value; s.refresh(); commit(); } }, () => !isReference(params[key]));
      input = s.el;
    } else {
      input = h("input", { id, testid: id, type: "text", autocomplete: "off", spellcheck: "false", placeholder: extra || "", value: params[key] || "",
        onInput: (e) => { params[key] = e.target.value; commit(); } });
    }
    return [h("label", { for: id }, label), input];
  };

  const draw = () => {
    const parts = [];
    if (HINTS[type]) parts.push(h("p.hint", HINTS[type]));
    if (FIELDS[type]) parts.push(h("div.form-grid", FIELDS[type].flatMap(field)));
    if (type === "oauth2") {
      const status = h("span.muted");
      parts.push(
        h("div.section-title", "Get a new access token"),
        h("div.form-grid", OAUTH2_GRANT.flatMap(field)),
        h("div.field-row", { style: "margin-top:10px" },
          h("button.btn", { testid: "oauth2-get-token", onClick: () => guard(async () => {
            status.textContent = "Requesting token…";
            try {
              const token = await api.post("/oauth2/token", { auth: makeAuth("oauth2", params), collection_id: opts.collectionId || null });
              params.accessToken = token.access_token;
              if (token.refresh_token) params.refreshToken = token.refresh_token;
              commit();
              draw();
              toast("Access token received", "ok");
            } finally {
              status.textContent = "";
            }
          }) }, "Get new access token"), status),
        h("p.hint", "Browser-based grants (authorization code, implicit) are not available here; paste the token above instead."));
    }
    clear(fieldsEl, parts);
  };
  draw();

  return h("div",
    h("div.field-row",
      h("label", { for: "auth-type", class: "muted" }, "Type"),
      h("select", { id: "auth-type", testid: "auth-type", onChange: (e) => {
        type = e.target.value;
        // keep values of fields that also exist in the new type (e.g. username/password)
        const allowed = new Set((FIELDS[type] || []).concat(type === "oauth2" ? OAUTH2_GRANT : []).map((f) => f[0]));
        params = Object.fromEntries(Object.entries(params).filter(([k]) => allowed.has(k)));
        commit();
        draw();
      } }, TYPES.filter(([v]) => v !== "inherit" || opts.allowInherit !== false).map(([v, l]) => h("option", { value: v, selected: v === type }, l)))),
    fieldsEl);
}

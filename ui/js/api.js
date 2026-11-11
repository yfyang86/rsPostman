// Thin client for the local rsPostman backend.

async function call(method, path, body, raw) {
  const opts = { method, headers: { "X-Requested-With": "rspostman" } };
  if (body !== undefined) {
    if (raw) {
      opts.body = body;
      opts.headers["Content-Type"] = "application/octet-stream";
    } else {
      opts.body = JSON.stringify(body);
      opts.headers["Content-Type"] = "application/json";
    }
  }
  let res;
  try {
    res = await fetch("/api" + path, opts);
  } catch (e) {
    throw new Error("Cannot reach the rsPostman backend. Is it still running?");
  }
  const type = res.headers.get("content-type") || "";
  const data = type.includes("application/json") ? await res.json() : await res.text();
  if (!res.ok) throw new Error((data && data.error) || `Request failed (${res.status})`);
  return data;
}

export const api = {
  get: (p) => call("GET", p),
  post: (p, b) => call("POST", p, b === undefined ? {} : b),
  put: (p, b) => call("PUT", p, b),
  del: (p) => call("DELETE", p),
  upload: (file) => call("POST", "/upload?name=" + encodeURIComponent(file.name), file, true),
};

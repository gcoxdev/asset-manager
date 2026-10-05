// The IPC contract, checked from both sides of the boundary.
//
// The UI names commands and their arguments as strings; nothing in the
// build connects them to the Rust signatures. This reads both: every
// `call("command", { ... })` in the frontend, parsed, against every
// `#[tauri::command]` in the backend and the list it registers. A renamed
// parameter, a misspelt key or a required argument left out fails here,
// not in front of a user. Object literals passed for a struct argument are
// checked field by field against the serde struct they become. (Result
// shapes are covered by the end-to-end mock-runtime test and the browser
// tests, which replay real responses.)

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
// Vite exports its bundler's parser (Rollup in 7, Rolldown in 8).
import { parseAst } from "vite";

const here = new URL("..", import.meta.url).pathname;
const SRC = join(here, "src");
const RUST = join(here, "src-tauri", "src");
const CRATES = join(here, "..", "..", "crates");

// Parameters Tauri supplies itself rather than reading from the caller.
const INJECTED = /^(tauri::)?(State|AppHandle|Window|WebviewWindow|Webview)\b|^tauri::(State|AppHandle|Window|WebviewWindow)/;

const camel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());

/** Split on commas not inside <>, (), [] or {}. */
function splitTop(text) {
  const out = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if ("<([{".includes(c)) depth++;
    else if (">)]}".includes(c)) depth--;
    else if (c === "," && depth === 0) {
      out.push(text.slice(start, i));
      start = i + 1;
    }
  }
  out.push(text.slice(start));
  return out.map((s) => s.trim()).filter(Boolean);
}

function rustCommands() {
  const commands = new Map();
  for (const file of readdirSync(RUST).filter((f) => f.endsWith(".rs"))) {
    const text = readFileSync(join(RUST, file), "utf8");
    const re = /#\[tauri::command\]\s*pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)\s*(?:<[^(]*>)?\s*\(/g;
    let m;
    while ((m = re.exec(text))) {
      // The parameter list runs to the matching close paren.
      let depth = 1;
      let i = re.lastIndex;
      for (; depth && i < text.length; i++) {
        if (text[i] === "(") depth++;
        else if (text[i] === ")") depth--;
      }
      const params = new Map();
      for (const param of splitTop(text.slice(re.lastIndex, i - 1))) {
        const [name, ...type] = param.split(":");
        const ty = type.join(":").trim();
        if (INJECTED.test(ty)) continue;
        params.set(camel(name.trim().replace(/^mut\s+/, "")), { optional: /^Option\s*</.test(ty), type: ty });
      }
      commands.set(m[1], { file, params });
    }
  }
  return commands;
}

function rustFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? rustFiles(join(dir, e.name)) : e.name.endsWith(".rs") ? [join(dir, e.name)] : []
  );
}

/** The innermost type name: `Option<am_storage::x::Form>` → `Form`. */
function typeName(ty) {
  return ty.replace(/^&/, "").trim().split("::").pop().replace(/<.*$/, "").trim();
}

/** Every struct that deserializes from IPC arguments, by name. */
function rustStructs() {
  const found = new Map();
  for (const file of [...rustFiles(RUST), ...readdirSync(CRATES).flatMap((c) => rustFiles(join(CRATES, c, "src")))]) {
    const text = readFileSync(file, "utf8");
    const re = /#\[derive\(([^)]*)\)\]\s*((?:#\[[^\]]*\]\s*)*)pub\s+struct\s+([A-Za-z0-9_]+)\s*\{/g;
    let m;
    while ((m = re.exec(text))) {
      if (!/\bDeserialize\b/.test(m[1])) continue;
      if (/rename_all/.test(m[2])) continue; // not checked: none of these cross IPC today
      const structDefault = /#\[serde\(default\)\]/.test(m[2]);
      let depth = 1;
      let i = re.lastIndex;
      for (; depth && i < text.length; i++) {
        if (text[i] === "{") depth++;
        else if (text[i] === "}") depth--;
      }
      const body = text.slice(re.lastIndex, i - 1).replace(/\/\/[^\n]*/g, "");
      const fields = new Map();
      const flatten = [];
      for (const f of body.matchAll(/((?:#\[[^\]]*\]\s*)*)pub\s+([a-z0-9_]+)\s*:\s*([^,]+(?:<[^>]*>[^,]*)?),?/g)) {
        const attrs = f[1];
        const ty = f[3].trim();
        if (/serde\(skip\)/.test(attrs)) continue;
        if (/serde\(flatten\)/.test(attrs)) { flatten.push(typeName(ty)); continue; }
        fields.set(f[2], { type: ty, optional: structDefault || /^Option\s*</.test(ty) || /serde\([^)]*default/.test(attrs) });
      }
      const list = found.get(m[3]) ?? [];
      list.push({ fields, flatten, where: file });
      found.set(m[3], list);
    }
  }
  return found;
}

function registered() {
  const lib = readFileSync(join(RUST, "lib.rs"), "utf8");
  const block = lib.slice(lib.indexOf("generate_handler!["), lib.indexOf("])", lib.indexOf("generate_handler![")));
  return new Set([...block.matchAll(/([a-z0-9_]+)::([a-z0-9_]+)/g)].map((m) => m[2]));
}

function jsFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? jsFiles(join(dir, e.name)) : e.name.endsWith(".js") ? [join(dir, e.name)] : []
  );
}

/** Every call("name", args) in the frontend, with the keys when args is a literal. */
function uiCalls() {
  const calls = [];
  for (const file of jsFiles(SRC)) {
    const code = readFileSync(file, "utf8");
    const ast = parseAst(code);
    const line = (pos) => code.slice(0, pos).split("\n").length;
    (function walk(node) {
      if (!node || typeof node.type !== "string") return;
      if (node.type === "CallExpression" && node.callee.type === "Identifier" && node.callee.name === "call") {
        const [name, args] = node.arguments;
        if (name?.type === "Literal" && typeof name.value === "string") {
          const where = `${file.slice(SRC.length + 1)}:${line(node.start)}`;
          if (!args) calls.push({ command: name.value, keys: [], where });
          else if (args.type === "ObjectExpression") {
            const spread = args.properties.some((p) => p.type === "SpreadElement");
            const props = args.properties.filter((p) => p.type === "Property");
            const keys = props.map((p) => p.key.name ?? p.key.value);
            const values = Object.fromEntries(props.map((p) => [p.key.name ?? p.key.value, p.value]));
            calls.push({ command: name.value, keys, values, spread, where });
          } else calls.push({ command: name.value, keys: null, where });
        }
      }
      for (const value of Object.values(node)) {
        if (Array.isArray(value)) value.forEach(walk);
        else if (value && typeof value === "object") walk(value);
      }
    })(ast);
  }
  return calls;
}

const commands = rustCommands();
const structs = rustStructs();

/** A struct's fields with any flattened ones merged in; null when unknown or ambiguous. */
function fieldsOf(name, seen = new Set()) {
  const list = structs.get(name);
  if (!list || list.length !== 1 || seen.has(name)) return null;
  seen.add(name);
  const fields = new Map(list[0].fields);
  for (const inner of list[0].flatten) {
    const more = fieldsOf(inner, seen);
    if (!more) return null; // a flattened map or unknown struct takes any key
    for (const [k, v] of more) fields.set(k, v);
  }
  return fields;
}

/** Problems with a literal `node` sent where Rust expects `ty`. */
function checkValue(node, ty, path) {
  const inner = ty.trim().match(/^Option\s*<(.*)>$/s);
  if (inner) return node.type === "Literal" && node.value === null ? [] : checkValue(node, inner[1], path);
  const vec = ty.trim().match(/^Vec\s*<(.*)>$/s);
  if (vec) {
    if (node.type !== "ArrayExpression") return [];
    return node.elements.flatMap((e, i) => (e ? checkValue(e, vec[1], `${path}[${i}]`) : []));
  }
  if (node.type !== "ObjectExpression") return [];
  const fields = fieldsOf(typeName(ty));
  if (!fields || node.properties.some((p) => p.type !== "Property")) return [];
  const sent = new Map(node.properties.map((p) => [p.key.name ?? p.key.value, p.value]));
  return [
    ...[...sent.keys()].filter((k) => !fields.has(k)).map((k) => `${path}.${k} is not a field of ${typeName(ty)}`),
    ...[...fields].filter(([k, f]) => !f.optional && !sent.has(k)).map(([k, f]) => `${path} is missing ${k}: ${f.type}`),
    ...[...sent].filter(([k]) => fields.has(k)).flatMap(([k, v]) => checkValue(v, fields.get(k).type, `${path}.${k}`)),
  ];
}
const handlers = registered();
const calls = uiCalls();

test("the parsers find what they should", () => {
  assert.ok(commands.size > 100, `found ${commands.size} commands`);
  assert.ok(calls.length > 100, `found ${calls.length} calls`);
  assert.deepEqual([...commands.get("split_asset").params.keys()], ["assetId", "quantity", "name"]);
});

test("every command the UI calls exists and is registered", () => {
  const problems = calls.flatMap((c) => [
    commands.has(c.command) ? null : `${c.where}: no command ${c.command}`,
    handlers.has(c.command) ? null : `${c.where}: ${c.command} is not registered in generate_handler!`,
  ]).filter(Boolean);
  assert.deepEqual(problems, []);
});

test("every argument the UI sends is one the command takes", () => {
  const problems = calls.filter((c) => c.keys && commands.has(c.command)).flatMap((c) => {
    const params = commands.get(c.command).params;
    return c.keys.filter((k) => !params.has(k)).map((k) => `${c.where}: ${c.command} takes no ${k} (it takes ${[...params.keys()].join(", ") || "nothing"})`);
  });
  assert.deepEqual(problems, []);
});

test("every required argument is sent", () => {
  const problems = calls.filter((c) => c.keys && !c.spread && commands.has(c.command)).flatMap((c) => {
    const params = commands.get(c.command).params;
    return [...params].filter(([k, p]) => !p.optional && !c.keys.includes(k)).map(([k, p]) => `${c.where}: ${c.command} needs ${k}: ${p.type}`);
  });
  assert.deepEqual(problems, []);
});

test("struct arguments sent as literals have the struct's fields", () => {
  const problems = calls.filter((c) => c.values && commands.has(c.command)).flatMap((c) => {
    const params = commands.get(c.command).params;
    return Object.entries(c.values)
      .filter(([k]) => params.has(k))
      .flatMap(([k, v]) => checkValue(v, params.get(k).type, k).map((p) => `${c.where}: ${c.command} ${p}`));
  });
  assert.deepEqual(problems, []);
});

test("struct checking reaches into the arguments", () => {
  const form = fieldsOf("ReportOptions");
  assert.ok(form?.has("include_evidence") && form.get("include_evidence").optional);
});

test("every registered command is a command", () => {
  assert.deepEqual([...handlers].filter((h) => !commands.has(h)), []);
});

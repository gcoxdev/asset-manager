import { cpSync, createReadStream, existsSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { dirname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const here = dirname(fileURLToPath(import.meta.url));
const pdfjs = resolve(here, "node_modules/pdfjs-dist");

// What pdf.js fetches at run time, served from our own origin so the CSP
// need not allow anything else: character maps for East Asian text, the 14
// standard fonts, and the plain-JS image decoders (the WebAssembly ones are
// not shipped — the viewer runs with useWasm off).
const PDFJS_DATA = {
  cmaps: () => true,
  standard_fonts: () => true,
  wasm: (name) => name.endsWith("_nowasm_fallback.js") || name.startsWith("LICENSE"),
};

function pdfjsData() {
  return {
    name: "pdfjs-data",
    configureServer(server) {
      server.middlewares.use("/pdfjs", (req, res, next) => {
        const path = normalize(decodeURIComponent(req.url.split("?")[0])).replace(/^[/\\]+/, "");
        const [dir, name] = path.split(/[/\\]/);
        const file = join(pdfjs, path);
        if (!PDFJS_DATA[dir]?.(name ?? "") || !file.startsWith(pdfjs) || !existsSync(file) || !statSync(file).isFile()) return next();
        createReadStream(file).pipe(res);
      });
    },
    writeBundle({ dir: out }) {
      for (const [dir, keep] of Object.entries(PDFJS_DATA)) {
        const target = join(out, "pdfjs", dir);
        mkdirSync(target, { recursive: true });
        for (const name of readdirSync(join(pdfjs, dir)).filter(keep)) cpSync(join(pdfjs, dir, name), join(target, name));
      }
    },
  };
}

export default defineConfig({
  root: "src",
  build: {
    outDir: "../web-dist",
    emptyOutDir: true,
    target: "es2022",
  },
  plugins: [pdfjsData()],
  clearScreen: false,
  server: { strictPort: true },
});

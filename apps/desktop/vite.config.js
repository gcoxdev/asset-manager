import { defineConfig } from "vite";

export default defineConfig({
  root: "src",
  build: {
    outDir: "../web-dist",
    emptyOutDir: true,
    target: "es2022",
  },
  clearScreen: false,
  server: { strictPort: true },
});

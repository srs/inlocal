import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, run `cargo run -- --no-open` in the repo root and `bun run dev` here.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://localhost:4321",
        changeOrigin: true,
      },
    },
  },
  build: { outDir: "dist", emptyOutDir: true },
});

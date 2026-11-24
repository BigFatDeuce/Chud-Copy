import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri attend un port fixe et gère lui-même la fenêtre : pas d'ouverture de navigateur.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1", watch: { ignored: ["**/src-tauri/**"] } },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    chunkSizeWarningLimit: 900,
    // La politique de sécurité n'autorise que des ressources locales, pas d'URI `data:`.
    assetsInlineLimit: 0,
  },
});

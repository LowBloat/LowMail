import { svelte } from "@sveltejs/vite-plugin-svelte";
import { caThemePlugin } from "@codeatlasdev/ui/vite";
import { defineConfig } from "vite";
import { theme } from "./src/theme.ts";

const host = process.env.TAURI_DEV_HOST;
const desktop = !!process.env.TAURI_ENV_PLATFORM;

// One build serves both front ends: the Tauri desktop app and the web
// server (`lowmail`), which embeds `dist/`. Relative asset paths let the web
// app live under any path behind a reverse proxy (/webmail/, for example).
export default defineConfig({
	base: "./",
	plugins: [caThemePlugin(theme), svelte()],
	clearScreen: false,
	server: {
		port: 1430,
		strictPort: true,
		host: host || "127.0.0.1",
		hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
		watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
		// Web development: the API comes from `bun run server` (port 8480).
		proxy: desktop ? undefined : { "/api": { target: "http://127.0.0.1:8480", changeOrigin: false } },
	},
	envPrefix: ["VITE_", "TAURI_ENV_*"],
	build: {
		target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome111" : "es2022",
		minify: !process.env.TAURI_ENV_DEBUG,
		sourcemap: !!process.env.TAURI_ENV_DEBUG,
		chunkSizeWarningLimit: 1500,
	},
});

// Drives LowMail on the Android emulator through the WebView DevTools port
// (adb forward tcp:9222): adds an account on the test mail server running on
// the host (10.0.2.2 from the emulator), reopens the app and waits for the
// inbox. Usage: bun android_smoke.mjs <out-dir>
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";

const out = process.argv[2];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function connect() {
	for (let i = 0; i < 30; i++) {
		const pages = await fetch("http://127.0.0.1:9222/json").then((r) => r.json()).catch(() => []);
		const page = pages.find((p) => p.type === "page" && p.webSocketDebuggerUrl);
		if (page) {
			const ws = new WebSocket(page.webSocketDebuggerUrl);
			await new Promise((resolve, reject) => ((ws.onopen = resolve), (ws.onerror = reject)));
			return ws;
		}
		await sleep(2000);
	}
	throw new Error("No WebView page to debug");
}

const ws = await connect();
let seq = 0;
const pending = new Map();
ws.onmessage = (m) => {
	const msg = JSON.parse(m.data);
	if (pending.has(msg.id)) pending.get(msg.id)(msg), pending.delete(msg.id);
};
const send = (method, params = {}) => new Promise((resolve) => (pending.set(++seq, resolve), ws.send(JSON.stringify({ id: seq, method, params }))));
async function run(code) {
	const r = await send("Runtime.evaluate", { expression: `(async () => { ${code} })()`, awaitPromise: true, returnByValue: true });
	if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 800));
	return r.result?.result?.value;
}
const screenshot = (name) => writeFileSync(`${out}/${name}`, execFileSync("adb", ["exec-out", "screencap", "-p"]));

const first = await run(`return { ua: navigator.userAgent, text: document.body.innerText.slice(0, 200) };`);
console.log("Start screen:", JSON.stringify(first));
if (!/Android/.test(first.ua)) throw new Error("Not the Android WebView");

const host = (port) => ({ host: "10.0.2.2", port, security: "none", acceptInvalidCerts: false });
const account = {
	id: "",
	label: "",
	name: "Alice",
	email: "alice@lowmail.test",
	username: "alice@lowmail.test",
	imap: host(1143),
	smtp: host(1025),
	sieve: host(4190),
	smtpAuth: true,
};
await run(`const inv = window.__TAURI_INTERNALS__.invoke;
	await inv("account_test", { account: ${JSON.stringify(account)}, password: "alice" });
	await inv("account_save", { account: ${JSON.stringify(account)}, password: "alice" });
	location.reload();`).catch(() => {});

let rows = 0;
for (let i = 0; i < 40 && rows === 0; i++) {
	await sleep(1500);
	rows = await run(`return document.querySelectorAll("[role=option]").length;`).catch(() => 0);
}
screenshot("2-inbox.png");
const inbox = await run(`return document.body.innerText.slice(0, 400);`);
console.log("Inbox:", JSON.stringify(inbox));
if (rows === 0) throw new Error("The inbox did not load");

// Open the first message: the reader must show its text.
await run(`document.querySelector("[role=option]").click();`);
await sleep(3000);
const frame = await run(`const f = document.querySelector("iframe.lm-frame"); return f ? f.contentDocument.body.innerText.slice(0, 200) : "";`);
console.log("Message:", JSON.stringify(frame));
if (!frame) throw new Error("The message did not open");
ws.close();
console.log(`OK: ${rows} messages listed on Android`);

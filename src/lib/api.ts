/**
 * The bridge to the Rust engine. The desktop app calls it through Tauri
 * commands; the browser through the web server's HTTP API. Both run the same
 * `lowmail_core::rpc::dispatch`, so the UI never needs to know which.
 */
import type { AccountConfig, AccountInfo, MailEvent, Upload } from "./types";

/** The native app (Tauri), on a computer or on Android. */
export const isDesktop = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
/** The native app on a phone or tablet: no window frame, no self-update. */
export const isMobile = isDesktop && /Android|iPhone|iPad|iPod/i.test(navigator.userAgent);

export type ErrorKind =
	| "auth"
	| "network"
	| "tls"
	| "notFound"
	| "invalid"
	| "server"
	| "unsupported"
	| "tooLarge"
	| "sessionExpired"
	| "internal";

export class ApiError extends Error {
	kind: ErrorKind;
	detail?: string;

	constructor(kind: ErrorKind, message: string, detail?: string) {
		super(message);
		this.kind = kind;
		this.detail = detail;
	}
}

function toApiError(e: unknown): ApiError {
	if (e instanceof ApiError) return e;
	if (e && typeof e === "object" && "kind" in e && "message" in e) {
		const o = e as { kind: ErrorKind; message: string; detail?: string };
		return new ApiError(o.kind, o.message, o.detail);
	}
	return new ApiError("internal", typeof e === "string" ? e : e instanceof Error ? e.message : "Unexpected error.");
}

/** The desktop account the calls are for; the web session has just one. */
let accountId: string | null = null;
let onSessionExpired: (() => void) | null = null;

export function setAccount(id: string | null) {
	accountId = id;
}

export function currentAccountId() {
	return accountId;
}

export function setSessionExpiredHandler(handler: () => void) {
	onSessionExpired = handler;
}

async function tauri() {
	return await import("@tauri-apps/api/core");
}

async function http<T>(path: string, init: RequestInit = {}): Promise<T> {
	let response: Response;
	try {
		response = await fetch(path, {
			credentials: "same-origin",
			...init,
			headers: { "x-lowmail": "1", ...(init.headers ?? {}) },
		});
	} catch (e) {
		throw new ApiError("network", "The LowMail server cannot be reached.", String(e));
	}
	const text = await response.text();
	let body: unknown = null;
	try {
		body = text ? JSON.parse(text) : null;
	} catch {
		body = null;
	}
	if (!response.ok) {
		const err = body && typeof body === "object" ? toApiError(body) : new ApiError("internal", `HTTP ${response.status}`);
		if (err.kind === "sessionExpired") onSessionExpired?.();
		throw err;
	}
	return body as T;
}

/** Calls an engine method (see crates/lowmail-core/src/rpc.rs). */
export async function call<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
	if (isDesktop) {
		const { invoke } = await tauri();
		try {
			return await invoke<T>("rpc", { accountId, method, params });
		} catch (e) {
			throw toApiError(e);
		}
	}
	return http<T>(`api/rpc/${method}`, {
		method: "POST",
		headers: { "content-type": "application/json" },
		body: JSON.stringify(params),
	});
}

// ---- web session ----

export interface WebConfig {
	productName: string;
	domain: string | null;
	version: string;
	filters: boolean;
}

export const web = {
	config: () => http<WebConfig>("api/config"),
	session: () => http<{ account: AccountInfo }>("api/session"),
	login: (email: string, password: string) =>
		http<{ account: AccountInfo }>("api/login", {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({ email, password }),
		}),
	logout: () => http<unknown>("api/logout", { method: "POST" }),
};

// ---- desktop accounts ----

export const desktop = {
	async invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
		const { invoke } = await tauri();
		try {
			return await invoke<T>(command, args);
		} catch (e) {
			throw toApiError(e);
		}
	},
	accounts: () => desktop.invoke<AccountConfig[]>("accounts_list"),
	saveAccount: (account: AccountConfig, password: string | null) => desktop.invoke<AccountConfig>("account_save", { account, password }),
	deleteAccount: (id: string) => desktop.invoke<void>("account_delete", { id }),
	testAccount: (account: AccountConfig, password: string | null) => desktop.invoke<void>("account_test", { account, password }),
	discover: (email: string) => desktop.invoke<AccountConfig>("account_discover", { email }),
	connect: (id: string, password: string | null = null) => desktop.invoke<AccountInfo>("account_connect", { id, password }),
	hasPassword: (id: string) => desktop.invoke<boolean>("account_has_password", { id }),
};

// ---- files ----

export async function upload(files: File[]): Promise<Upload[]> {
	if (isDesktop) {
		const { invoke } = await tauri();
		const out: Upload[] = [];
		for (const file of files) {
			const bytes = new Uint8Array(await file.arrayBuffer());
			try {
				out.push(
					await invoke<Upload>("upload", bytes, {
						headers: {
							"x-account": accountId ?? "",
							"x-filename": encodeURIComponent(file.name),
							"x-content-type": file.type || "",
						},
					}),
				);
			} catch (e) {
				throw toApiError(e);
			}
		}
		return out;
	}
	const form = new FormData();
	for (const f of files) form.append("file", f, f.name);
	const res = await http<{ uploads: Upload[] }>("api/upload", { method: "POST", body: form });
	return res.uploads;
}

/** Attachment chosen from disk with the native dialog (desktop only). */
export async function uploadFromDisk(): Promise<Upload[]> {
	return desktop.invoke<Upload[]>("upload_pick", { accountId });
}

function query(params: Record<string, string | number>) {
	return new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)])).toString();
}

export function attachmentUrl(folder: string, uid: number, part: string, inline = false) {
	return `api/download/attachment?${query({ folder, uid, part, inline: inline ? 1 : 0 })}`;
}

function clickDownload(url: string) {
	const a = document.createElement("a");
	a.href = url;
	a.rel = "noopener";
	a.download = "";
	document.body.appendChild(a);
	a.click();
	a.remove();
}

export async function saveAttachment(folder: string, uid: number, part: string) {
	if (isDesktop) return desktop.invoke<boolean>("attachment_save", { accountId, folder, uid, part });
	clickDownload(attachmentUrl(folder, uid, part));
	return true;
}

/** Opens an attachment with the system's app (desktop) or in a new tab. */
export async function openAttachment(folder: string, uid: number, part: string) {
	if (isDesktop) return desktop.invoke<void>("attachment_open", { accountId, folder, uid, part });
	window.open(attachmentUrl(folder, uid, part, true), "_blank", "noopener");
}

export async function saveMessage(folder: string, uid: number) {
	if (isDesktop) return desktop.invoke<boolean>("message_save", { accountId, folder, uid });
	clickDownload(`api/download/message?${query({ folder, uid })}`);
	return true;
}

/** Saves text as a file (contacts export, for example). */
export async function saveText(filename: string, text: string, type = "text/plain") {
	if (isDesktop) return desktop.invoke<boolean>("text_save", { filename, text });
	const url = URL.createObjectURL(new Blob([text], { type }));
	const a = document.createElement("a");
	a.href = url;
	a.download = filename;
	a.click();
	setTimeout(() => URL.revokeObjectURL(url), 10_000);
	return true;
}

export async function openExternal(url: string) {
	if (isDesktop) {
		const { openUrl } = await import("@tauri-apps/plugin-opener");
		await openUrl(url);
	} else {
		window.open(url, "_blank", "noopener,noreferrer");
	}
}

// ---- events ----

/** Listens to new mail and changes. Returns a function that stops listening. */
export async function listen(onEvent: (event: MailEvent) => void, onError?: () => void): Promise<() => void> {
	if (isDesktop) {
		const { listen } = await import("@tauri-apps/api/event");
		const id = accountId;
		await desktop.invoke("watch", { accountId: id });
		return await listen<{ accountId: string; event: MailEvent }>("mail-event", (e) => {
			if (e.payload.accountId === id) onEvent(e.payload.event);
		});
	}
	let source: EventSource | null = null;
	let stopped = false;
	let retry: ReturnType<typeof setTimeout> | undefined;
	const open = () => {
		source = new EventSource("api/events");
		source.onmessage = (m) => {
			try {
				onEvent(JSON.parse(m.data));
			} catch {
				// Comments and malformed events are ignored.
			}
		};
		source.onerror = () => {
			source?.close();
			onError?.();
			if (!stopped) retry = setTimeout(open, 10_000);
		};
	};
	open();
	return () => {
		stopped = true;
		clearTimeout(retry);
		source?.close();
	};
}

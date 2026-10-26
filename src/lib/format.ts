import { i18n, t } from "./i18n/index.svelte";
import type { Address, Folder, FolderRole } from "./types";

function sameDay(a: Date, b: Date) {
	return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** Short date for the message list: time today, day and month this year, full date before. */
export function listDate(ms: number): string {
	if (!ms) return "";
	const d = new Date(ms);
	const now = new Date();
	const loc = i18n.dateLocale;
	if (sameDay(d, now)) return d.toLocaleTimeString(loc, { hour: "2-digit", minute: "2-digit" });
	const yesterday = new Date(now);
	yesterday.setDate(now.getDate() - 1);
	if (sameDay(d, yesterday)) return t("date.yesterday");
	if (now.getTime() - d.getTime() < 6 * 86_400_000) return d.toLocaleDateString(loc, { weekday: "short" });
	if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString(loc, { day: "numeric", month: "short" });
	return d.toLocaleDateString(loc, { day: "2-digit", month: "2-digit", year: "numeric" });
}

export function fullDate(ms: number): string {
	if (!ms) return "";
	return new Date(ms).toLocaleString(i18n.dateLocale, {
		weekday: "short",
		day: "numeric",
		month: "short",
		year: "numeric",
		hour: "2-digit",
		minute: "2-digit",
	});
}

export function size(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	const units = ["KB", "MB", "GB", "TB"];
	let v = bytes / 1024;
	let i = 0;
	while (v >= 1024 && i < units.length - 1) {
		v /= 1024;
		i++;
	}
	return `${v.toLocaleString(i18n.dateLocale, { maximumFractionDigits: v < 10 ? 1 : 0 })} ${units[i]}`;
}

export function displayName(a: Address | undefined | null): string {
	if (!a) return "";
	return a.name?.trim() || a.email;
}

export function formatAddress(a: Address): string {
	const name = a.name?.trim();
	if (!name) return a.email;
	const quoted = /[",;<>@()]/.test(name) ? `"${name.replace(/"/g, "'")}"` : name;
	return `${quoted} <${a.email}>`;
}

export function initials(a: Address | undefined | null): string {
	const text = displayName(a).replace(/["'<>]/g, "").trim();
	if (!text) return "?";
	const words = text.split(/[\s._@-]+/).filter(Boolean);
	if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
	return (words[0][0] + words[words.length - 1][0]).toUpperCase();
}

/** Parses "Name <a@b>, c@d; e@f" into addresses; invalid parts are returned apart. */
export function parseAddresses(input: string): { valid: Address[]; invalid: string[] } {
	const valid: Address[] = [];
	const invalid: string[] = [];
	// Split on commas and semicolons outside quotes and angle brackets.
	const parts: string[] = [];
	let cur = "";
	let quote = false;
	let angle = false;
	for (const ch of input) {
		if (ch === '"') quote = !quote;
		else if (ch === "<") angle = true;
		else if (ch === ">") angle = false;
		if ((ch === "," || ch === ";" || ch === "\n") && !quote && !angle) {
			parts.push(cur);
			cur = "";
		} else cur += ch;
	}
	parts.push(cur);
	for (const raw of parts.map((p) => p.trim()).filter(Boolean)) {
		const m = raw.match(/^(.*?)\s*<\s*([^<>\s]+@[^<>\s]+)\s*>$/);
		let a: Address | null = null;
		if (m) a = { name: m[1].replace(/^"|"$/g, "").trim() || null, email: m[2] };
		else if (/^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/.test(raw)) a = { name: null, email: raw };
		if (a && isEmail(a.email)) valid.push(a);
		else invalid.push(raw);
	}
	return { valid, invalid };
}

export function isEmail(s: string): boolean {
	return /^[^\s@<>()",;]+@[^\s@<>()",;]+\.[^\s@<>()",;]{2,}$/.test(s.trim());
}

export function folderLabel(f: Pick<Folder, "role" | "name">): string {
	return f.role ? t(`folder.${f.role}`) : f.name;
}

/** The folder with its parents, as the folder list shows them ("Caixa de entrada / spam"). */
export function folderPathLabel(f: Folder, folders: Folder[]): string {
	const names = [folderLabel(f)];
	const seen = new Set([f.path]);
	let parent = f.parent;
	while (parent && !seen.has(parent)) {
		seen.add(parent);
		const p = folders.find((x) => x.path === parent);
		if (!p) break;
		names.unshift(folderLabel(p));
		parent = p.parent;
	}
	return names.join(" / ");
}

export function roleIcon(role: FolderRole | null): string {
	return role ?? "folder";
}

export function plural(key: string, n: number, vars: Record<string, string | number> = {}) {
	return t(key, { n, ...vars });
}

/** Escapes text for HTML. */
export function escapeHtml(s: string): string {
	return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

export function textToHtml(text: string): string {
	return escapeHtml(text).replace(/\r?\n/g, "<br>");
}

/** Plain text of an HTML fragment. */
export function htmlToText(html: string): string {
	const doc = new DOMParser().parseFromString(html, "text/html");
	doc.querySelectorAll("br").forEach((br) => br.replaceWith("\n"));
	doc.querySelectorAll("p, div, li, tr, h1, h2, h3, h4, blockquote").forEach((el) => el.append("\n"));
	return (doc.body.textContent ?? "").replace(/\n{3,}/g, "\n\n").trim();
}
